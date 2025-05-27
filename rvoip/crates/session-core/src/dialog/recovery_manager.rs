use tracing::{debug, error, info, warn};
use std::time::SystemTime;

use super::manager::DialogManager;
use super::dialog_id::DialogId;
use super::dialog_state::DialogState;
use super::recovery::{RecoveryMetrics, perform_recovery_process, needs_recovery};
use crate::errors::{Error, ErrorContext, ErrorCategory, ErrorSeverity, RecoveryAction};
use crate::events::SessionEvent;
use crate::session::SessionId;
use crate::{dialog_not_found_error};

impl DialogManager {
    /// Initiate dialog recovery after detecting a network failure
    pub async fn recover_dialog(&self, dialog_id: &DialogId, reason: &str) -> Result<(), Error> {
        // Check if the dialog exists
        let dialog_opt = self.dialogs.get_mut(dialog_id);
        if dialog_opt.is_none() {
            return Err(dialog_not_found_error(dialog_id));
        }
        
        // Get the dialog and check if it can be recovered
        let mut dialog = dialog_opt.unwrap();
        if dialog.state != DialogState::Confirmed && dialog.state != DialogState::Early {
            return Err(Error::InvalidDialogState {
                current: dialog.state.to_string(),
                expected: "Confirmed or Early".to_string(),
                context: ErrorContext::default()
            });
        }
        
        // Check if we have a last known remote address
        if dialog.last_known_remote_addr.is_none() {
            return Err(Error::MissingDialogData {
                context: ErrorContext::default().with_message(
                    "Dialog does not have a last known remote address"
                )
            });
        }
        
        // Check if circuit breaker is active
        {
            let metrics = self.recovery_metrics.read().await;
            if metrics.circuit_breaker_open {
                if let Some(reset_time) = metrics.last_circuit_breaker_reset {
                    if let Ok(elapsed) = SystemTime::now().duration_since(reset_time) {
                        if elapsed < self.recovery_config.circuit_breaker_reset_period {
                            warn!("Dialog recovery circuit breaker is open, rejecting recovery for dialog {}", dialog_id);
                            // Use NetworkUnreachable which is more appropriate for circuit breaker pattern
                            let wait_time = self.recovery_config.circuit_breaker_reset_period.checked_sub(elapsed)
                                .unwrap_or_default();
                            return Err(Error::NetworkUnreachable(
                                format!("Circuit breaker active for dialog {}", dialog_id),
                                ErrorContext {
                                    category: ErrorCategory::Network,
                                    severity: ErrorSeverity::Warning,
                                    recovery: RecoveryAction::Wait(wait_time),
                                    retryable: true,
                                    dialog_id: Some(dialog_id.to_string()),
                                    timestamp: SystemTime::now(),
                                    details: Some(format!("Circuit breaker active for {} more seconds", 
                                        wait_time.as_secs())),
                                    ..Default::default()
                                }
                            ));
                        } else {
                            // Reset circuit breaker if enough time has passed
                            drop(metrics); // Release the read lock
                            let mut metrics = self.recovery_metrics.write().await;
                            metrics.reset_circuit_breaker();
                            info!("Dialog recovery circuit breaker reset after timeout period");
                        }
                    }
                }
            }
        }
        
        // Put the dialog into recovery mode
        dialog.enter_recovery_mode(reason);
        
        // Get the session ID for events
        let session_id = self.get_session_for_dialog(dialog_id)
            .ok_or_else(|| Error::session_not_found("No session found for dialog"))?;
        
        // Make a clone of the dialog ID before releasing the lock
        let dialog_id_clone = dialog_id.clone();
        
        // Release the lock before firing events
        drop(dialog);
        
        // Publish a specific recovery started event
        self.event_bus.publish(SessionEvent::DialogRecoveryStarted {
            session_id: session_id.clone(),
            dialog_id: dialog_id.clone(),
            reason: reason.to_string(),
        });
        
        if self.run_recovery_in_background {
            // Start the recovery process in a background task
            let manager = self.clone();
            tokio::spawn(async move {
                manager.execute_recovery_process(&dialog_id_clone).await;
            });
        } else {
            // For testing, run recovery process synchronously
            self.execute_recovery_process(dialog_id).await;
        }
        
        Ok(())
    }
    
    /// Execute the dialog recovery process (retry logic, etc.)
    async fn execute_recovery_process(&self, dialog_id: &DialogId) {
        debug!("Starting recovery process for dialog {}", dialog_id);
        
        // Get a reference to the dialog
        let mut dialog_opt = self.dialogs.get_mut(dialog_id);
        if dialog_opt.is_none() {
            debug!("Dialog {} not found for recovery", dialog_id);
            return;
        }
        
        // Prepare to run the recovery process
        let transport = self.transaction_manager.transport();
        let config = &self.recovery_config;
        
        // We need a mutable reference to metrics, but tokio's RwLock is async
        let metrics_arc = self.recovery_metrics.clone();
        
        // Get session ID for events
        let session_id = self.get_session_for_dialog(dialog_id);
        
        // Setup dialog and transport
        let mut dialog = dialog_opt.unwrap();
        
        // Create event callback for logging and events
        let event_bus = self.event_bus.clone();
        let dialog_id_clone = dialog_id.clone();
        let session_id_clone = session_id.clone();
        let event_callback = move |event: super::recovery::RecoveryEvent| {
            match &event {
                super::recovery::RecoveryEvent::AttemptStarted { attempt, max_attempts } => {
                    info!("Starting recovery attempt {} of {} for dialog {}", 
                        attempt, max_attempts, dialog_id_clone);
                    
                    // Emit event through event bus if needed
                    if let Some(session_id) = &session_id_clone {
                        event_bus.publish(crate::events::SessionEvent::Custom {
                            session_id: session_id.clone(),
                            event_type: "recovery_attempt_started".to_string(),
                            data: serde_json::json!({
                                "dialog_id": dialog_id_clone.to_string(),
                                "attempt": attempt,
                                "max_attempts": max_attempts
                            }),
                        });
                    }
                },
                super::recovery::RecoveryEvent::AttemptSucceeded { time_ms } => {
                    info!("Dialog {} recovery succeeded in {}ms", dialog_id_clone, time_ms);
                },
                super::recovery::RecoveryEvent::AttemptFailed { attempt, reason, is_timeout } => {
                    if *is_timeout {
                        warn!("Dialog {} recovery attempt {} timed out", dialog_id_clone, attempt);
                    } else {
                        warn!("Dialog {} recovery attempt {} failed: {}", 
                            dialog_id_clone, attempt, reason);
                    }
                    
                    // Emit event through event bus if needed
                    if let Some(session_id) = &session_id_clone {
                        event_bus.publish(crate::events::SessionEvent::Custom {
                            session_id: session_id.clone(),
                            event_type: "recovery_attempt_failed".to_string(),
                            data: serde_json::json!({
                                "dialog_id": dialog_id_clone.to_string(),
                                "attempt": attempt,
                                "reason": reason,
                                "is_timeout": is_timeout
                            }),
                        });
                    }
                },
                super::recovery::RecoveryEvent::RetryDelay { delay_ms } => {
                    debug!("Waiting {}ms before next recovery attempt for dialog {}", 
                        delay_ms, dialog_id_clone);
                }
            }
        };
        
        // Run the recovery process
        // Note: We must take mutable access to metrics inside the perform_recovery_process function
        // since tokio::RwLock requires an .await after lock()
        let recovery_result = perform_recovery_process(
            &mut dialog,
            transport.as_ref(),
            config,
            &metrics_arc,
            event_callback
        ).await;
        
        // Drop the dialog reference before handling the result
        // to prevent deadlocks with other locks
        drop(dialog);
        
        // Process the recovery result
        match recovery_result {
            super::recovery::RecoveryResult::Success { recovery_time_ms } => {
                info!("Dialog {} successfully recovered in {}ms", dialog_id, recovery_time_ms);
                
                // No need to call mark_recovery_successful here as it was done inside perform_recovery_process
                // But we still need to emit the recovery completed event
                if let Some(session_id) = session_id {
                    self.event_bus.publish(SessionEvent::DialogRecoveryCompleted {
                        session_id,
                        dialog_id: dialog_id.clone(),
                        success: true,
                    });
                }
            },
            super::recovery::RecoveryResult::Failure { reason, activate_circuit_breaker } => {
                warn!("Dialog {} recovery failed: {}", dialog_id, reason);
                
                // Activate circuit breaker if needed
                if activate_circuit_breaker {
                    // Need to use async lock acquire with await point
                    let mut metrics = self.recovery_metrics.write().await;
                    metrics.open_circuit_breaker();
                    warn!("Dialog recovery circuit breaker opened after consecutive failures");
                    drop(metrics); // Explicitly drop the lock
                }
                
                // Emit recovery completed event
                if let Some(session_id) = session_id {
                    // Emit dialog state changed event
                    self.event_bus.publish(SessionEvent::DialogStateChanged {
                        session_id: session_id.clone(),
                        dialog_id: dialog_id.clone(),
                        previous: DialogState::Recovering,
                        current: DialogState::Terminated,
                    });
                    
                    // Emit specific recovery failed event
                    self.event_bus.publish(SessionEvent::DialogRecoveryCompleted {
                        session_id: session_id.clone(),
                        dialog_id: dialog_id.clone(),
                        success: false,
                    });
                    
                    // Emit dialog/session terminated event
                    self.event_bus.publish(SessionEvent::Terminated {
                        session_id,
                        reason: format!("Recovery failed: {}", reason),
                    });
                }
            },
            super::recovery::RecoveryResult::Aborted { reason } => {
                warn!("Dialog {} recovery aborted: {}", dialog_id, reason);
                
                // Emit recovery aborted event
                if let Some(session_id) = session_id {
                    self.event_bus.publish(crate::events::SessionEvent::Custom {
                        session_id,
                        event_type: "recovery_aborted".to_string(),
                        data: serde_json::json!({
                            "dialog_id": dialog_id.to_string(),
                            "reason": reason
                        }),
                    });
                }
            }
        }
    }
    
    /// Check if a dialog needs recovery based on network failure
    pub async fn needs_recovery(&self, dialog_id: &DialogId) -> bool {
        let dialog_opt = self.dialogs.get(dialog_id);
        if dialog_opt.is_none() {
            return false;
        }
        
        let dialog = dialog_opt.unwrap();
        let config = &self.recovery_config;
        let metrics = self.recovery_metrics.read().await;
        needs_recovery(&dialog, config, &metrics)
    }
    
    /// Get current recovery metrics
    pub async fn recovery_metrics(&self) -> RecoveryMetrics {
        self.recovery_metrics.read().await.clone()
    }
    
    /// Test-only method to bypass the recovery process entirely and directly set the dialog state
    #[cfg(test)]
    pub async fn test_simulate_recovery(&self, dialog_id: &DialogId, success: bool) -> Result<(), Error> {
        // Get the dialog and set it to Recovering state first
        {
            let mut dialog = self.dialogs.get_mut(dialog_id)
                .ok_or_else(|| dialog_not_found_error(dialog_id))?;
            
            dialog.state = DialogState::Recovering;
            dialog.recovery_reason = Some("Test simulated recovery".to_string());
            dialog.recovery_start_time = Some(SystemTime::now());
        }
        
        // Small delay to let tasks process
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        
        // Update dialog based on success parameter
        self.update_dialog_property(dialog_id, |dialog| {
            if success {
                super::recovery::complete_recovery(dialog);
            } else {
                super::recovery::abandon_recovery(dialog);
            }
        })?;
        
        // Emit appropriate events
        let session_id = self.get_session_for_dialog(dialog_id);
        if let Some(session_id) = session_id {
            if success {
                self.event_bus.publish(SessionEvent::DialogRecoveryCompleted {
                    session_id,
                    dialog_id: dialog_id.clone(),
                    success: true,
                });
            } else {
                self.event_bus.publish(SessionEvent::DialogRecoveryCompleted {
                    session_id: session_id.clone(),
                    dialog_id: dialog_id.clone(),
                    success: false,
                });
                
                self.event_bus.publish(SessionEvent::Terminated {
                    session_id,
                    reason: "Simulated recovery failure".to_string(),
                });
            }
        }
        
        Ok(())
    }
    
    // Methods to support testing without exposing internal fields directly
    
    /// Get a dialog's state (primarily for testing)
    pub fn get_dialog_state(&self, dialog_id: &DialogId) -> Result<DialogState, Error> {
        match self.dialogs.get(dialog_id) {
            Some(dialog) => Ok(dialog.state.clone()),
            None => Err(dialog_not_found_error(dialog_id))
        }
    }
    
    /// Update a dialog's property for testing
    pub fn update_dialog_property(&self, dialog_id: &DialogId, 
                                  updater: impl FnOnce(&mut super::dialog_impl::Dialog)) -> Result<(), Error> {
        match self.dialogs.get_mut(dialog_id) {
            Some(mut dialog) => {
                updater(&mut dialog);
                Ok(())
            },
            None => Err(dialog_not_found_error(dialog_id))
        }
    }
    
    /// Get a dialog's property (for testing)
    pub fn get_dialog_property<T: Clone>(&self, dialog_id: &DialogId, 
                                        getter: impl FnOnce(&super::dialog_impl::Dialog) -> T) -> Result<T, Error> {
        match self.dialogs.get(dialog_id) {
            Some(dialog) => Ok(getter(&dialog)),
            None => Err(dialog_not_found_error(dialog_id))
        }
    }
    
    /// Check if a transaction is associated with a dialog (for testing)
    pub fn is_transaction_associated(&self, transaction_id: &rvoip_transaction_core::TransactionKey, dialog_id: &DialogId) -> bool {
        // We can't use match self.transaction_to_dialog.get(transaction_id) due to type issues,
        // so check for key existence first
        if self.transaction_to_dialog.contains_key(transaction_id) {
            if let Some(stored_id) = self.transaction_to_dialog.get(transaction_id) {
                return *stored_id == *dialog_id;
            }
        }
        false
    }
} 