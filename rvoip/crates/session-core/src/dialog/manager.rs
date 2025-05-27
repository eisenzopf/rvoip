use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error};
use std::time::SystemTime;

use rvoip_transaction_core::{
    TransactionManager, 
    TransactionEvent, 
    TransactionKey,
};

use super::dialog_state::DialogState;
use super::transaction_coordination::TransactionCoordinator;
use super::call_lifecycle::CallLifecycleCoordinator;
use crate::errors::{Error, ErrorContext, ErrorCategory, ErrorSeverity, RecoveryAction};
use crate::events::{EventBus, SessionEvent};
use crate::session::SessionId;
use crate::media::MediaManager;
use crate::{dialog_not_found_error};

use super::dialog_id::DialogId;
use super::dialog_impl::Dialog;
use super::recovery::{RecoveryConfig, RecoveryMetrics};

// Constants for channel sizing and buffer management
const DEFAULT_EVENT_CHANNEL_SIZE: usize = 100;

/// Manager for SIP dialogs that integrates with the transaction layer
#[derive(Clone)]
pub struct DialogManager {
    /// Active dialogs by ID
    pub(super) dialogs: DashMap<DialogId, Dialog>,
    
    /// Dialog lookup by SIP dialog identifier tuple (call-id, local-tag, remote-tag)
    pub(super) dialog_lookup: DashMap<(String, String, String), DialogId>,
    
    /// DialogId mapped to SessionId for session references
    pub(super) dialog_to_session: DashMap<DialogId, SessionId>,
    
    /// Transaction manager reference
    pub(super) transaction_manager: Arc<TransactionManager>,
    
    /// Transaction to Dialog mapping
    pub(super) transaction_to_dialog: DashMap<TransactionKey, DialogId>,
    
    /// Track which transactions we've already subscribed to to avoid duplicate subscriptions
    pub(super) subscribed_transactions: DashMap<TransactionKey, bool>,
    
    /// Main event channel for distributing transaction events
    pub(super) event_sender: mpsc::Sender<TransactionEvent>,
    
    /// Event bus for dialog events
    pub(super) event_bus: EventBus,
    
    /// For testing purposes - whether to run recovery in background
    pub(super) run_recovery_in_background: bool,
    
    /// Recovery configuration
    pub(super) recovery_config: RecoveryConfig,
    
    /// Recovery metrics
    pub(super) recovery_metrics: Arc<RwLock<RecoveryMetrics>>,
    
    /// Call lifecycle coordinator for automatic call handling
    pub(super) call_lifecycle_coordinator: Option<Arc<CallLifecycleCoordinator>>,
}

impl DialogManager {
    /// Create a new dialog manager
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
    ) -> Self {
        Self::new_with_full_config(
            transaction_manager,
            event_bus,
            true,
            RecoveryConfig::default(),
        )
    }
    
    /// Create a new dialog manager with custom recovery configuration
    pub fn new_with_recovery_config(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
        recovery_config: RecoveryConfig,
    ) -> Self {
        Self::new_with_full_config(
            transaction_manager,
            event_bus,
            true,
            recovery_config,
        )
    }
    
    /// Create a new dialog manager with a specified recovery mode (for testing)
    pub fn new_with_recovery_mode(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
        run_recovery_in_background: bool,
    ) -> Self {
        Self::new_with_full_config(
            transaction_manager,
            event_bus,
            run_recovery_in_background,
            RecoveryConfig::default(),
        )
    }
    
    /// Create a fully customized dialog manager (for testing)
    pub fn new_with_full_config(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
        run_recovery_in_background: bool,
        recovery_config: RecoveryConfig,
    ) -> Self {
        // Create the main event channel
        let (event_sender, mut event_receiver) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        
        let dialogs = DashMap::new();
        let dialog_lookup = DashMap::new();
        let dialog_to_session = DashMap::new();
        let transaction_to_dialog = DashMap::new();
        let subscribed_transactions = DashMap::new();
        let recovery_metrics = Arc::new(RwLock::new(RecoveryMetrics::default()));
        
        // Create the dialog manager
        let dialog_manager = Self {
            dialogs,
            dialog_lookup,
            dialog_to_session,
            transaction_manager: transaction_manager.clone(),
            transaction_to_dialog,
            subscribed_transactions,
            event_sender,
            event_bus,
            run_recovery_in_background,
            recovery_config,
            recovery_metrics,
            call_lifecycle_coordinator: None,
        };
        
        // Start the event processor for the event_receiver
        let dm = dialog_manager.clone();
        tokio::spawn(async move {
            while let Some(event) = event_receiver.recv().await {
                dm.process_transaction_event(event).await;
            }
        });
        
        dialog_manager
    }
    
    /// Subscribe to transaction events and start processing them
    pub async fn start(&self) -> mpsc::Receiver<TransactionEvent> {
        // Set up direct event forwarding 
        let dialog_manager = self.clone();
        
        // Create a single task that directly subscribes to ALL transactions
        // This avoids the polling approach and batch processing entirely
        tokio::spawn(async move {
            let tx_manager = dialog_manager.transaction_manager.clone();
            
            // Subscribe to ALL transaction events - we'll filter them as needed
            let mut all_events_rx = tx_manager.subscribe();
            
            // Process events directly
            while let Some(event) = all_events_rx.recv().await {
                if let Err(e) = dialog_manager.event_sender.send(event.clone()).await {
                    error!("Failed to forward transaction event: {}", e);
                    break;
                }
                
                // Process the event directly as well to avoid any delays
                dialog_manager.process_transaction_event(event).await;
            }
            
            debug!("Main transaction event subscription task terminated");
        });
        
        // Return a subscription for the caller to use if needed
        self.transaction_manager.subscribe()
    }
    
    /// Get the current number of active dialogs
    pub fn dialog_count(&self) -> usize {
        self.dialogs.len()
    }
    
    /// Get a dialog by ID
    pub fn get_dialog(&self, dialog_id: &DialogId) -> Result<Dialog, Error> {
        self.dialogs.get(dialog_id)
            .map(|d| d.clone())
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))
    }
    
    /// Terminate a dialog
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> Result<(), Error> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot terminate - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))?;
            
        dialog.terminate();
        Ok(())
    }
    
    /// Remove terminated dialogs
    pub fn cleanup_terminated(&self) -> usize {
        let mut count = 0;
        
        let terminated_dialogs: Vec<_> = self.dialogs.iter()
            .filter(|d| d.is_terminated())
            .map(|d| d.id.clone())
            .collect();
        
        for dialog_id in terminated_dialogs {
            if let Some((_, dialog)) = self.dialogs.remove(&dialog_id) {
                count += 1;
                
                // Remove from the lookup tables
                // Get the dialog tuple directly from the dialog
                let call_id = &dialog.call_id;
                if let (Some(local_tag), Some(remote_tag)) = (&dialog.local_tag, &dialog.remote_tag) {
                    let tuple = (call_id.clone(), local_tag.clone(), remote_tag.clone());
                    self.dialog_lookup.remove(&tuple);
                }
                
                self.dialog_to_session.remove(&dialog_id);
                
                // Remove transaction associations
                let txs_to_remove: Vec<_> = self.transaction_to_dialog.iter()
                    .filter(|e| e.value().clone() == dialog_id)
                    .map(|e| e.key().clone())
                    .collect();
                
                for tx_id in txs_to_remove {
                    self.transaction_to_dialog.remove(&tx_id);
                }
            }
        }
        
        count
    }
    
    /// Stop the dialog manager and clean up all resources
    pub async fn stop(&self) -> Result<(), Error> {
        debug!("Stopping dialog manager");
        
        // Check if we have any active dialogs
        let active_dialogs = self.dialog_count();
        if active_dialogs > 0 {
            debug!("Stopping dialog manager with {} active dialogs", active_dialogs);
            
            // Get all dialog IDs
            let dialog_ids: Vec<DialogId> = self.dialogs.iter()
                .map(|entry| entry.key().clone())
                .collect();
            
            // Terminate each dialog with timeout
            let terminate_futures = dialog_ids.iter().map(|dialog_id| {
                let dialog_id = dialog_id.clone();
                let dm = self.clone();
                
                async move {
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(500), 
                        dm.terminate_dialog(&dialog_id)
                    ).await {
                        Ok(Ok(_)) => true,
                        _ => {
                            debug!("Failed to terminate dialog {} during shutdown", dialog_id);
                            false
                        }
                    }
                }
            });
            
            // Execute all terminations concurrently with an overall timeout
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                futures::future::join_all(terminate_futures)
            ).await {
                Ok(results) => {
                    let success_count = results.iter().filter(|&&success| success).count();
                    let failed_count = results.len() - success_count;
                    
                    if failed_count > 0 {
                        debug!("Failed to terminate {} dialogs during shutdown", failed_count);
                    }
                    
                    debug!("Successfully terminated {} of {} dialogs", 
                          success_count, dialog_ids.len());
                },
                Err(_) => {
                    debug!("Timeout during dialog termination, forcing cleanup");
                }
            }
        }
        
        // Force cleanup of any remaining resources
        self.cleanup_all();
        
        debug!("Dialog manager stopped successfully");
        Ok(())
    }
    
    /// Clean up all resources
    fn cleanup_all(&self) {
        // Clear all mappings
        self.dialogs.clear();
        self.dialog_lookup.clear();
        self.dialog_to_session.clear();
        self.transaction_to_dialog.clear();
        self.subscribed_transactions.clear();
    }
    
    // Helper method to find a session associated with a transaction
    pub(super) fn find_session_for_transaction(&self, transaction_id: &TransactionKey) -> Option<SessionId> {
        // First, look up the dialog ID
        let dialog_id = match self.transaction_to_dialog.get(transaction_id) {
            Some(ref_val) => {
                let dialog_id = ref_val.clone();
                drop(ref_val);
                dialog_id
            },
            None => return None
        };
        
        // Now look up the session ID for this dialog
        match self.dialog_to_session.get(&dialog_id) {
            Some(ref_val) => {
                let session_id = ref_val.clone();
                drop(ref_val);
                Some(session_id)
            },
            None => None
        }
    }
    
    /// Get the session ID associated with a dialog
    pub(super) fn get_session_for_dialog(&self, dialog_id: &DialogId) -> Option<SessionId> {
        self.dialog_to_session.get(dialog_id).map(|id| id.clone())
    }
    
    /// Create a new dialog manager with call lifecycle coordinator
    pub fn new_with_call_coordinator(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
        media_manager: Arc<MediaManager>,
    ) -> Self {
        // Create transaction coordinator
        let transaction_coordinator = TransactionCoordinator::new(transaction_manager.clone());
        
        // Create call lifecycle coordinator
        let call_lifecycle_coordinator = Arc::new(CallLifecycleCoordinator::new(
            transaction_coordinator,
            media_manager,
        ));
        
        let mut dialog_manager = Self::new_with_full_config(
            transaction_manager,
            event_bus,
            true,
            RecoveryConfig::default(),
        );
        
        // Set the call lifecycle coordinator
        dialog_manager.call_lifecycle_coordinator = Some(call_lifecycle_coordinator);
        
        dialog_manager
    }
    
    /// Set the call lifecycle coordinator
    pub fn set_call_lifecycle_coordinator(&mut self, coordinator: Arc<CallLifecycleCoordinator>) {
        self.call_lifecycle_coordinator = Some(coordinator);
    }
} 