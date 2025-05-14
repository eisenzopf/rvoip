//! Manages the lifecycle of SIP transaction timers and dispatches timer events.
//!
//! The [`TimerManager`] is responsible for:
//! - Registering and unregistering transactions that require timer services.
//! - Starting one-shot timer tasks that, upon expiration, send an [`InternalTransactionCommand::Timer`]
//!   event to the associated transaction.
//! - Holding timer settings applicable to its operations.
//!
//! Note: This `TimerManager` provides a mechanism for scheduling a single notification after a
//! specified duration. For timers that require periodic firing or complex backoff strategies
//! (like RFC 3261 Timer A or E), the transaction itself, upon receiving a timer event,
//! is responsible for performing its action (e.g., retransmission) and then requesting the
//! `TimerManager` to start a new timer with the next appropriate duration.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::sleep;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, trace, warn};

use crate::transaction::{TransactionKey, InternalTransactionCommand};
// Ensure TimerSettings is correctly imported if it was moved to super::types
use super::types::{TimerSettings, TimerType};
// Timer struct from types.rs is not directly used by TimerManager methods but is related contextually.

/// Manages active timers for SIP transactions.
///
/// When a timer fires, the `TimerManager` sends an `InternalTransactionCommand::Timer` message
/// to the `mpsc::Sender<InternalTransactionCommand>` that was registered for that transaction.
/// It does not directly manage `Timer` struct instances but rather the spawned tasks for each active timer.
#[derive(Debug)]
pub struct TimerManager {
    /// Stores sender channels for `InternalTransactionCommand`s, keyed by `TransactionKey`.
    /// Used to notify a specific transaction when one of its timers fires.
    transaction_channels: Arc<Mutex<HashMap<TransactionKey, mpsc::Sender<InternalTransactionCommand>>>>,
    /// Configuration settings for timers, such as default durations (T1, T2 etc.).
    /// While `TimerManager` itself mostly deals with given durations, these settings might inform
    /// those durations if not provided directly to `start_timer` or by a `TimerFactory`.
    settings: TimerSettings,
}

impl TimerManager {
    /// Creates a new `TimerManager`.
    ///
    /// # Arguments
    /// * `settings` - Optional [`TimerSettings`]. If `None`, default settings are used.
    pub fn new(settings: Option<TimerSettings>) -> Self {
        Self {
            transaction_channels: Arc::new(Mutex::new(HashMap::new())),
            settings: settings.unwrap_or_default(),
        }
    }
    
    /// Registers a transaction with the `TimerManager`.
    ///
    /// This allows the `TimerManager` to send timer-fired events to the transaction via the provided `command_tx` channel.
    /// Typically called when a new transaction is created and needs timer supervision.
    ///
    /// # Arguments
    /// * `transaction_id` - The [`TransactionKey`] of the transaction to register.
    /// * `command_tx` - The `mpsc::Sender` channel for sending [`InternalTransactionCommand`]s to the transaction.
    pub async fn register_transaction(
        &self,
        transaction_id: TransactionKey,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let mut channels = self.transaction_channels.lock().await;
        if channels.insert(transaction_id.clone(), command_tx).is_some() {
            warn!(id=%transaction_id, "Transaction channel replaced for already registered transaction.");
        }
        trace!(id=%transaction_id, "Transaction registered with TimerManager.");
    }
    
    /// Unregisters a transaction from the `TimerManager`.
    ///
    /// After unregistering, the transaction will no longer receive timer events. Active timer tasks
    /// for this transaction might still complete their sleep, but they will not be able to send an event.
    /// Typically called when a transaction terminates.
    ///
    /// # Arguments
    /// * `transaction_id` - The [`TransactionKey`] of the transaction to unregister.
    pub async fn unregister_transaction(&self, transaction_id: &TransactionKey) {
        let mut channels = self.transaction_channels.lock().await;
        if channels.remove(transaction_id).is_some() {
            trace!(id=%transaction_id, "Transaction unregistered from TimerManager.");
        } else {
            trace!(id=%transaction_id, "Attempted to unregister a non-existent transaction.");
        }
    }
    
    /// Starts a one-shot timer for a specific transaction.
    ///
    /// A new asynchronous task is spawned that will sleep for the given `duration`.
    /// Upon waking, it sends an [`InternalTransactionCommand::Timer`] containing the `timer_type`
    /// (as a string) to the registered channel for the `transaction_id`.
    ///
    /// If the transaction is unregistered before the timer fires, or if its command channel
    /// is closed, the event delivery will fail silently or log an error, respectively.
    ///
    /// # Arguments
    /// * `transaction_id` - The [`TransactionKey`] of the transaction this timer belongs to.
    /// * `timer_type` - The [`TimerType`] of this timer, used for generating the event payload.
    /// * `duration` - The [`Duration`] for which the timer should sleep before firing.
    ///
    /// # Returns
    /// `Ok(JoinHandle<()>)` for the spawned timer task. The caller can use this handle
    /// to await the timer task's completion or to abort it (though aborting is not
    /// explicitly managed by `TimerManager` beyond providing the handle).
    /// Returns `crate::error::Error` if the underlying transaction channel is not found *immediately*
    /// (though the current implementation spawns and checks later).
    /// The primary error source would be if `transaction_channels.lock()` fails, which is unlikely.
    /// The current implementation always returns Ok, as the check happens in spawned task.
    pub async fn start_timer(
        &self,
        transaction_id: TransactionKey,
        timer_type: TimerType,
        duration: Duration,
    ) -> Result<JoinHandle<()>, crate::error::Error> { // Consider if crate::error::Error is appropriate here.
        let transaction_channels_clone = self.transaction_channels.clone();
        
        // Check if transaction channel exists *before* spawning to provide immediate feedback if possible.
        // However, this adds a lock acquisition before spawning. For high-frequency timers, this might be a concern.
        // The current design does the check *after* sleep, which is fine for eventual consistency.
        // If we want to return an error if the channel is not *currently* registered, we'd do:
        // { // Scope for the lock
        //     let channels_guard = transaction_channels_clone.lock().await;
        //     if !channels_guard.contains_key(&transaction_id) {
        //         warn!(id = %transaction_id, timer = %timer_type, "Cannot start timer, transaction not registered.");
        //         // How to return an error that fits crate::error::Error type?
        //         // For now, let's stick to the original behavior of spawning and checking later.
        //     }
        // }

        let handle = tokio::spawn(async move {
            trace!(id=%transaction_id, timer=%timer_type, duration=?duration, "Timer task started.");
            
            sleep(duration).await;
            
            let channels_guard = transaction_channels_clone.lock().await;
            if let Some(cmd_tx) = channels_guard.get(&transaction_id) {
                let timer_event_payload = timer_type.to_string();
                trace!(id=%transaction_id, timer=%timer_type, "Timer fired. Attempting to send event.");
                if let Err(e) = cmd_tx.send(InternalTransactionCommand::Timer(timer_event_payload.clone())).await {
                    // This error typically means the receiver (transaction) has been dropped/terminated.
                    debug!(id=%transaction_id, timer=%timer_event_payload, error=%e, "Failed to send timer event (receiver dropped).");
                } else {
                    debug!(id=%transaction_id, timer=%timer_event_payload, "Timer event sent successfully.");
                }
            } else {
                // Transaction was unregistered before timer fired.
                trace!(id=%transaction_id, timer=%timer_type, "Timer fired, but transaction no longer registered.");
            }
        });
        
        Ok(handle) // tokio::spawn itself doesn't typically return a Result in this form.
    }
    
    /// Returns a reference to the [`TimerSettings`] used by this manager.
    pub fn settings(&self) -> &TimerSettings {
        &self.settings
    }
}

/// Provides a default `TimerManager` with default [`TimerSettings`].
impl Default for TimerManager {
    fn default() -> Self {
        Self::new(None)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::TransactionKey;
    use rvoip_sip_core::Method;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    // Helper to create a dummy TransactionKey for tests
    fn dummy_tm_tx_key(name: &str) -> TransactionKey {
        TransactionKey::new(format!("branch-manager-{}", name), Method::Options, false)
    }

    #[test]
    fn timer_manager_new_and_default() {
        let settings = TimerSettings { t1: Duration::from_millis(100), ..Default::default() };
        let manager = TimerManager::new(Some(settings.clone()));
        assert_eq!(manager.settings(), &settings);
        assert!(manager.transaction_channels.try_lock().unwrap().is_empty());

        let default_manager = TimerManager::default();
        assert_eq!(*default_manager.settings(), TimerSettings::default());
    }

    #[tokio::test]
    async fn timer_manager_register_unregister_transaction() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("reg_unreg");
        let (cmd_tx, _) = mpsc::channel(1);

        manager.register_transaction(tx_key.clone(), cmd_tx).await;
        assert!(manager.transaction_channels.lock().await.contains_key(&tx_key));

        manager.unregister_transaction(&tx_key).await;
        assert!(!manager.transaction_channels.lock().await.contains_key(&tx_key));
        
        // Test unregistering a non-existent key (should not panic)
        manager.unregister_transaction(&dummy_tm_tx_key("non_existent")).await;
    }

    #[tokio::test]
    async fn timer_manager_start_timer_sends_event() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("send_event");
        let (cmd_tx, mut cmd_rx) = mpsc::channel(10); // Increased buffer for safety

        manager.register_transaction(tx_key.clone(), cmd_tx).await;

        let timer_duration = Duration::from_millis(50);
        let timer_type = TimerType::Custom;

        let handle = manager.start_timer(tx_key.clone(), timer_type, timer_duration).await.unwrap();

        // Wait for the timer event
        match timeout(timer_duration + Duration::from_millis(50), cmd_rx.recv()).await {
            Ok(Some(InternalTransactionCommand::Timer(payload))) => {
                assert_eq!(payload, timer_type.to_string());
            }
            Ok(Some(other_cmd)) => panic!("Received unexpected command: {:?}", other_cmd),
            Ok(None) => panic!("Command channel closed unexpectedly"),
            Err(_) => panic!("Timeout waiting for timer event"),
        }
        
        handle.await.expect("Timer task panicked");
    }

    #[tokio::test]
    async fn timer_manager_timer_fires_for_unregistered_transaction() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("unregistered_fire");
        let (cmd_tx, mut cmd_rx) = mpsc::channel(1);

        manager.register_transaction(tx_key.clone(), cmd_tx).await;

        let timer_duration = Duration::from_millis(20);
        let handle = manager.start_timer(tx_key.clone(), TimerType::A, timer_duration).await.unwrap();
        
        // Unregister immediately after starting
        manager.unregister_transaction(&tx_key).await;

        // The timer task will run, but it shouldn't find the channel to send the event.
        // We check that no event is received.
        match timeout(timer_duration + Duration::from_millis(50), cmd_rx.recv()).await {
            Ok(Some(_)) => panic!("Should not have received a timer event for unregistered transaction"),
            Ok(None) => { /* Channel closed or empty, expected */ },
            Err(_) => { /* Timeout, also expected as no event should arrive */ trace!("Timeout as expected for unregistered timer test.") },
        }
        handle.await.expect("Timer task for unregistered tx panicked");
    }

    #[tokio::test]
    async fn timer_manager_timer_receiver_dropped() {
        let manager = TimerManager::new(None);
        let tx_key = dummy_tm_tx_key("rx_dropped");
        let (cmd_tx, cmd_rx) = mpsc::channel(1);

        manager.register_transaction(tx_key.clone(), cmd_tx).await;
        drop(cmd_rx); // Drop the receiver

        let timer_duration = Duration::from_millis(20);
        // The start_timer itself should succeed.
        let handle = manager.start_timer(tx_key.clone(), TimerType::B, timer_duration).await.unwrap();
        
        // The spawned task will attempt to send, but it will fail because the receiver is dropped.
        // This should be handled gracefully within the task (e.g., logged error).
        // We just await the handle to ensure the task completes without panicking.
        match timeout(timer_duration + Duration::from_millis(50), handle).await {
            Ok(Ok(())) => { /* Task completed */ },
            Ok(Err(e)) => panic!("Timer task join error: {}", e),
            Err(_) => panic!("Timeout waiting for timer task to complete after receiver dropped"),
        }
    }
    
    #[test]
    fn timer_manager_settings_accessor() {
        let custom_settings = TimerSettings { t1: Duration::from_secs(10), ..Default::default() };
        let manager = TimerManager::new(Some(custom_settings.clone()));
        assert_eq!(manager.settings(), &custom_settings);
    }
} 