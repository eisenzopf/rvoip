//! TransactionManager messaging, query, and lifecycle operations

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::pin::Pin;
use std::future::Future;

use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn, trace};

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::TypedHeader;
use rvoip_sip_transport::{Transport, TransportEvent};

use crate::transaction::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionAsync, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand,
};
use crate::transaction::state::TransactionLifecycle;
use crate::transaction::client::{
    ClientTransaction,
    TransactionExt,
};
use crate::transaction::runner::HasLifecycle;
use crate::transaction::server::ServerTransaction;
use crate::transaction::timer::{Timer, TimerSettings};
use crate::transaction::utils::transaction_key_from_message;

use super::TransactionManager;

impl TransactionManager {
    pub async fn send_request(&self, transaction_id: &TransactionKey) -> Result<()> {
        debug!(%transaction_id, "TransactionManager::send_request - sending request");
        
        // We need to get the transaction and clone only when needed
        let mut locked_txs = self.client_transactions.lock().await;
        
        // Get a reference to the transaction to determine its type
        let tx = match locked_txs.get_mut(transaction_id) {
            Some(tx) => tx,
            None => {
                debug!(%transaction_id, "TransactionManager::send_request - transaction not found");
                return Err(Error::transaction_not_found(transaction_id.clone(), "send_request - transaction not found"));
            }
        };
        debug!(%transaction_id, kind=?tx.kind(), state=?tx.state(), "TransactionManager::send_request - found transaction");
        
        // Remember initial state to detect quick state transitions
        let initial_state = tx.state();
        
        // First subscribe to events BEFORE initiating the transaction
        // so we don't miss any events that happen during initiation
        let mut event_rx = self.subscribe();
        
        // Use the TransactionExt trait to safely downcast
        use crate::transaction::client::TransactionExt;
        
        if let Some(client_tx) = tx.as_client_transaction() {
            debug!(%transaction_id, "TransactionManager::send_request - initiating client transaction");
            
            // Issue the initiate command
            let result = client_tx.initiate().await;
            debug!(%transaction_id, success=?result.is_ok(), "TransactionManager::send_request - initiate result");
            
            // If initiate() returned an error, return it immediately
            if let Err(e) = result {
                debug!(%transaction_id, error=%e, "TransactionManager::send_request - initiate failed immediately");
                return Err(e);
            }
            
            // Check transaction state immediately after initiate
            let current_state = tx.state();
            if current_state == TransactionState::Terminated {
                // Transaction terminated immediately - likely due to transport error
                debug!(%transaction_id, "Transaction terminated immediately during initiate - likely transport error");
                return Err(Error::transport_error(
                    rvoip_sip_transport::Error::ProtocolError("Transaction terminated immediately".into()),
                    "Failed to send request - transaction terminated immediately"
                ));
            }
            
            // Release lock to allow transaction processing
            drop(locked_txs);
            
            // Now wait for a short time to catch any asynchronous errors
            // We'll use a timeout to avoid hanging if no events are received
            let timeout_duration = tokio::time::Duration::from_millis(100);
            
            match tokio::time::timeout(timeout_duration, async {
                // Wait for events until timeout
                while let Some(event) = event_rx.recv().await {
                    match event {
                        TransactionEvent::TransportError { transaction_id: tx_id, .. } if tx_id == *transaction_id => {
                            debug!(%transaction_id, "Received TransportError event");
                            return Err(Error::transport_error(
                                rvoip_sip_transport::Error::ProtocolError("Transport error during request send".into()),
                                "Failed to send request - transport error"
                            ));
                        },
                        TransactionEvent::StateChanged { transaction_id: tx_id, previous_state, new_state } 
                            if tx_id == *transaction_id => {
                            debug!(%transaction_id, previous=?previous_state, new=?new_state, "Transaction state changed");
                            
                            // If transaction moved directly to Terminated state
                            if new_state == TransactionState::Terminated && 
                               (previous_state == TransactionState::Initial || 
                                previous_state == TransactionState::Calling || 
                                previous_state == TransactionState::Trying) {
                                
                                debug!(%transaction_id, "Transaction moved to Terminated state - likely transport error");
                                return Err(Error::transport_error(
                                    rvoip_sip_transport::Error::ProtocolError("Transaction terminated unexpectedly".into()),
                                    "Failed to send request - transaction terminated"
                                ));
                            }
                        },
                        _ => {} // Ignore other events
                    }
                }
                
                // Check final transaction state
                let locked_txs = self.client_transactions.lock().await;
                if let Some(tx) = locked_txs.get(transaction_id) {
                    let final_state = tx.state();
                    if final_state == TransactionState::Terminated {
                        debug!(%transaction_id, "Transaction is terminated after events processed");
                        return Err(Error::transport_error(
                            rvoip_sip_transport::Error::ProtocolError("Transaction terminated after processing".into()),
                            "Failed to send request - transaction terminated"
                        ));
                    }
                } else {
                    // Transaction was removed
                    debug!(%transaction_id, "Transaction was removed - likely due to termination");
                    return Err(Error::transport_error(
                        rvoip_sip_transport::Error::ProtocolError("Transaction was removed".into()),
                        "Failed to send request - transaction removed"
                    ));
                }
                
                Ok(())
            }).await {
                // Timeout occurred
                Err(_) => {
                    // Check one more time if the transaction still exists or has terminated
                    let locked_txs = self.client_transactions.lock().await;
                    if let Some(tx) = locked_txs.get(transaction_id) {
                        let final_state = tx.state();
                        if final_state == TransactionState::Terminated {
                            debug!(%transaction_id, "Transaction terminated after timeout");
                            return Err(Error::transport_error(
                                rvoip_sip_transport::Error::ProtocolError("Transaction terminated after timeout".into()),
                                "Failed to send request - transaction terminated"
                            ));
                        }
                        
                        // If we still have a transaction and it's not terminated, assume it's okay
                        debug!(%transaction_id, state=?final_state, "Transaction still exists and is not terminated after timeout");
                        Ok(())
                    } else {
                        // Transaction was removed
                        debug!(%transaction_id, "Transaction was removed after timeout");
                        Err(Error::transport_error(
                            rvoip_sip_transport::Error::ProtocolError("Transaction was removed after timeout".into()),
                            "Failed to send request - transaction removed"
                        ))
                    }
                },
                // Got a result from the event processing
                Ok(result) => result,
            }
        } else {
            debug!(%transaction_id, "TransactionManager::send_request - failed to downcast to client transaction");
            Err(Error::Other("Failed to downcast to client transaction".to_string()))
        }
    }

    /// Sends a response through a server transaction.
    ///
    /// This method sends a SIP response through an existing server transaction,
    /// which will handle retransmissions and state transitions according to
    /// RFC 3261 rules.
    ///
    /// ## Transaction State Transitions
    ///
    /// This method can trigger the following state transitions:
    /// - INVITE server with provisional response: Proceeding → Proceeding
    /// - INVITE server with final response: Proceeding → Completed  
    /// - Non-INVITE server with provisional response: Trying/Proceeding → Proceeding
    /// - Non-INVITE server with final response: Trying/Proceeding → Completed
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.2.1: INVITE server transaction response handling
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction response handling
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the server transaction
    /// * `response` - The SIP response to send
    ///
    /// # Returns
    /// * `Result<()>` - Success or error if the response cannot be sent
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_core::{Response, StatusCode};
    /// # use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// # use rvoip_dialog_core::transaction::TransactionManager;
    /// # use rvoip_dialog_core::transaction::TransactionKey;
    /// # async fn example(
    /// #    manager: &TransactionManager,
    /// #    tx_id: &TransactionKey,
    /// #    request: &rvoip_sip_core::Request
    /// # ) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a 200 OK response
    /// let response = SimpleResponseBuilder::response_from_request(
    ///     request,
    ///     StatusCode::Ok,
    ///     Some("OK")
    /// ).build();
    ///
    /// // Send the response through the transaction
    /// manager.send_response(tx_id, response).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> Result<()> {
        // We need to get the transaction and clone only when needed
        let mut locked_txs = self.server_transactions.lock().await;
        
        // Get a reference to the transaction to determine its type
        let tx = match locked_txs.get_mut(transaction_id) {
            Some(tx) => tx,
            None => {
                return Err(Error::transaction_not_found(transaction_id.clone(), "send_response - transaction not found"));
            }
        };
        
        // Use the TransactionExt trait to safely downcast
        use crate::transaction::server::TransactionExt;
        
        if let Some(server_tx) = tx.as_server_transaction() {
            server_tx.send_response(response).await
        } else {
            Err(Error::Other("Failed to downcast to server transaction".to_string()))
        }
    }

    /// Checks if a transaction with the given ID exists.
    ///
    /// This method looks for the transaction in both client and server
    /// transaction collections. It's useful for verifying that a transaction
    /// exists before attempting operations on it.
    ///
    /// # Arguments
    /// * `transaction_id` - The transaction ID to check
    ///
    /// # Returns
    /// * `bool` - True if the transaction exists, false otherwise
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_dialog_core::transaction::TransactionManager;
    /// # use rvoip_dialog_core::transaction::TransactionKey;
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) {
    /// if manager.transaction_exists(tx_id).await {
    ///     println!("Transaction {} exists", tx_id);
    /// } else {
    ///     println!("Transaction {} not found", tx_id);
    /// }
    /// # }
    /// ```
    pub async fn transaction_exists(&self, transaction_id: &TransactionKey) -> bool {
        let client_exists = {
            let client_txs = self.client_transactions.lock().await;
            client_txs.contains_key(transaction_id)
        };
        
        if client_exists {
            return true;
        }
        
        let server_exists = {
            let server_txs = self.server_transactions.lock().await;
            server_txs.contains_key(transaction_id)
        };
        
        server_exists
    }

    /// Gets the current state of a transaction.
    ///
    /// This method retrieves the current state of a transaction according to
    /// the state machines defined in RFC 3261. The state determines what
    /// operations are valid and how the transaction will respond to messages.
    ///
    /// ## Transaction States
    ///
    /// The possible states are:
    /// - **Initial**: Transaction created but not started
    /// - **Calling**: INVITE client waiting for response
    /// - **Trying**: Non-INVITE client waiting for response
    /// - **Proceeding**: Received provisional response, waiting for final
    /// - **Completed**: Received final response, waiting for reliability
    /// - **Terminated**: Transaction is done
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1: INVITE client transaction states
    /// - RFC 3261 Section 17.1.2: Non-INVITE client transaction states
    /// - RFC 3261 Section 17.2.1: INVITE server transaction states
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction states
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the transaction
    ///
    /// # Returns
    /// * `Result<TransactionState>` - The transaction state or error if not found
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_dialog_core::transaction::TransactionManager;
    /// # use rvoip_dialog_core::transaction::{TransactionKey, TransactionState};
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) -> Result<(), Box<dyn std::error::Error>> {
    /// let state = manager.transaction_state(tx_id).await?;
    /// 
    /// match state {
    ///     TransactionState::Proceeding => println!("Transaction is in Proceeding state"),
    ///     TransactionState::Completed => println!("Transaction is in Completed state"),
    ///     TransactionState::Terminated => println!("Transaction is terminated"),
    ///     _ => println!("Transaction is in state: {:?}", state),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transaction_state(&self, transaction_id: &TransactionKey) -> Result<TransactionState> {
        // Try client transactions first
        {
            let client_txs = self.client_transactions.lock().await;
            if let Some(tx) = client_txs.get(transaction_id) {
                return Ok(tx.state());
            }
        }
        
        // Try server transactions
        {
            let server_txs = self.server_transactions.lock().await;
            if let Some(tx) = server_txs.get(transaction_id) {
                return Ok(tx.state());
            }
        }
        
        // Transaction not found
        Err(Error::transaction_not_found(transaction_id.clone(), "transaction_state - transaction not found"))
    }

    /// Gets the transaction type (kind) for the specified transaction.
    ///
    /// This method returns the type of transaction as defined in RFC 3261:
    /// - INVITE client transaction (ICT)
    /// - Non-INVITE client transaction (NICT)
    /// - INVITE server transaction (IST) 
    /// - Non-INVITE server transaction (NIST)
    ///
    /// Knowing the transaction kind is important because each type follows
    /// different state machines and behavior rules.
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17: Four transaction types with different state machines
    /// - RFC 3261 Section 17.1: Client transaction types
    /// - RFC 3261 Section 17.2: Server transaction types
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the transaction
    ///
    /// # Returns
    /// * `Result<TransactionKind>` - The transaction kind or error if not found
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_dialog_core::transaction::TransactionManager;
    /// # use rvoip_dialog_core::transaction::{TransactionKey, TransactionKind};
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) -> Result<(), Box<dyn std::error::Error>> {
    /// let kind = manager.transaction_kind(tx_id).await?;
    /// 
    /// match kind {
    ///     TransactionKind::InviteClient => println!("This is an INVITE client transaction"),
    ///     TransactionKind::NonInviteClient => println!("This is a non-INVITE client transaction"),
    ///     TransactionKind::InviteServer => println!("This is an INVITE server transaction"),
    ///     TransactionKind::NonInviteServer => println!("This is a non-INVITE server transaction"),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transaction_kind(&self, transaction_id: &TransactionKey) -> Result<TransactionKind> {
        let client_txs = self.client_transactions.lock().await;
        if let Some(tx) = client_txs.get(transaction_id) {
            return Ok(tx.kind());
        }

        let server_txs = self.server_transactions.lock().await;
        if let Some(tx) = server_txs.get(transaction_id) {
            return Ok(tx.kind());
        }

        Err(Error::transaction_not_found(transaction_id.clone(), "transaction kind lookup failed"))
    }

    /// Gets a list of all active transaction IDs.
    ///
    /// This method returns separate lists of client and server transaction IDs,
    /// which can be useful for monitoring, debugging, or cleanup operations.
    ///
    /// # Returns
    /// * `(Vec<TransactionKey>, Vec<TransactionKey>)` - Client and server transaction IDs
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_dialog_core::transaction::TransactionManager;
    /// # async fn example(manager: &TransactionManager) {
    /// let (client_txs, server_txs) = manager.active_transactions().await;
    /// 
    /// println!("Active client transactions: {}", client_txs.len());
    /// println!("Active server transactions: {}", server_txs.len());
    /// 
    /// // Process each transaction ID
    /// for tx_id in client_txs {
    ///     println!("Client transaction: {}", tx_id);
    /// }
    /// # }
    /// ```
    pub async fn active_transactions(&self) -> (Vec<TransactionKey>, Vec<TransactionKey>) {
        let client_txs = self.client_transactions.lock().await;
        let server_txs = self.server_transactions.lock().await;

        (
            client_txs.keys().cloned().collect(),
            server_txs.keys().cloned().collect(),
        )
    }

    /// Subscribe to the shutdown broadcast signal.
    ///
    /// Spawned tasks can use the returned receiver to exit gracefully when
    /// the transaction manager is shutting down.
    pub fn subscribe_shutdown(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Gets a reference to the transport layer used by this transaction manager.
    ///
    /// This method provides access to the underlying transport layer,
    /// which can be useful for operations outside the transaction layer.
    ///
    /// # Returns
    /// * `Arc<dyn Transport>` - The transport layer
    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }

    /// Subscribe to events from all transactions.
    ///
    /// This method creates a new subscription to all transaction events.
    /// The returned receiver will get all events regardless of which transaction
    /// generated them. This is useful for monitoring or logging all transaction activity.
    ///
    /// # Returns
    /// * `mpsc::Receiver<TransactionEvent>` - The event receiver
    pub fn subscribe(&self) -> mpsc::Receiver<TransactionEvent> {
        let (tx, rx) = mpsc::channel(100);
        
        std::mem::drop(tokio::spawn({
            let subscribers = self.event_subscribers.clone();
            let next_subscriber_id = self.next_subscriber_id.clone();
            
            async move {
                let mut subscriber_id = next_subscriber_id.lock().await;
                let id = *subscriber_id;
                *subscriber_id += 1;
                
                // Add to global subscribers list
                let mut subs = subscribers.lock().await;
                subs.push(tx);
                
                debug!("Added global subscriber with ID {}", id);
            }
        }));
        
        rx
    }
    
    /// Subscribe to events from a specific transaction.
    ///
    /// This method creates a subscription to events from a single transaction.
    /// The returned receiver will only get events from the specified transaction,
    /// reducing noise and unnecessary event processing.
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the transaction to subscribe to
    ///
    /// # Returns
    /// * `Result<mpsc::Receiver<TransactionEvent>>` - The event receiver
    pub async fn subscribe_to_transaction(&self, transaction_id: &TransactionKey) -> Result<mpsc::Receiver<TransactionEvent>> {
        // Validate that transaction exists
        if !self.transaction_exists(transaction_id).await {
            return Err(Error::transaction_not_found(transaction_id.clone(), "subscribe_to_transaction - transaction not found"));
        }
        
        let (tx, rx) = mpsc::channel(100);
        
        // Register the subscription
        let subscriber_id = {
            let mut next_id = self.next_subscriber_id.lock().await;
            let id = *next_id;
            *next_id += 1;
            id
        };
        
        // Add to global subscribers list
        {
            let mut subs = self.event_subscribers.lock().await;
            subs.push(tx);
        }
        
        // Add to transaction-specific mapping
        {
            let mut tx_to_subs = self.transaction_to_subscribers.lock().await;
            
            // Create entry if it doesn't exist
            let subscriber_list = tx_to_subs.entry(transaction_id.clone())
                .or_insert_with(Vec::new);
            
            // Add this subscriber
            subscriber_list.push(subscriber_id);
        }
        
        // Add to subscriber-to-transactions mapping
        {
            let mut sub_to_txs = self.subscriber_to_transactions.lock().await;
            
            // Create entry if it doesn't exist
            let transaction_list = sub_to_txs.entry(subscriber_id)
                .or_insert_with(Vec::new);
            
            // Add this transaction
            transaction_list.push(transaction_id.clone());
        }
        
        debug!(%transaction_id, subscriber_id, "Added transaction-specific subscriber");
        
        Ok(rx)
    }
    
    /// Subscribe to events from multiple transactions.
    ///
    /// This method creates a subscription to events from multiple transactions.
    /// The returned receiver will only get events from the specified transactions.
    ///
    /// # Arguments
    /// * `transaction_ids` - The IDs of the transactions to subscribe to
    ///
    /// # Returns
    /// * `Result<mpsc::Receiver<TransactionEvent>>` - The event receiver
    pub async fn subscribe_to_transactions(&self, transaction_ids: &[TransactionKey]) -> Result<mpsc::Receiver<TransactionEvent>> {
        // Validate that all transactions exist
        for tx_id in transaction_ids {
            if !self.transaction_exists(tx_id).await {
                return Err(Error::transaction_not_found(tx_id.clone(), "subscribe_to_transactions - transaction not found"));
            }
        }
        
        let (tx, rx) = mpsc::channel(100);
        
        // Register the subscription
        let subscriber_id = {
            let mut next_id = self.next_subscriber_id.lock().await;
            let id = *next_id;
            *next_id += 1;
            id
        };
        
        // Add to global subscribers list
        {
            let mut subs = self.event_subscribers.lock().await;
            subs.push(tx);
        }
        
        // Add to transaction-specific mapping
        {
            let mut tx_to_subs = self.transaction_to_subscribers.lock().await;
            
            for tx_id in transaction_ids {
                // Create entry if it doesn't exist
                let subscriber_list = tx_to_subs.entry(tx_id.clone())
                    .or_insert_with(Vec::new);
                
                // Add this subscriber
                subscriber_list.push(subscriber_id);
            }
        }
        
        // Add to subscriber-to-transactions mapping
        {
            let mut sub_to_txs = self.subscriber_to_transactions.lock().await;
            
            // Create entry if it doesn't exist
            let transaction_list = sub_to_txs.entry(subscriber_id)
                .or_insert_with(Vec::new);
            
            // Add these transactions
            for tx_id in transaction_ids {
                transaction_list.push(tx_id.clone());
            }
        }
        
        debug!(subscriber_id, transaction_count = transaction_ids.len(), "Added multi-transaction subscriber");
        
        Ok(rx)
    }

    /// Shutdown the transaction manager gracefully - BOTTOM-UP
    /// 
    /// This performs a graceful shutdown in BOTTOM-UP order:
    /// 1. Close the transport layer (UDP) first
    /// 2. Stop the message processing loop
    /// 3. Drain any remaining messages
    /// 4. Clear active transactions
    /// 5. Clear event subscribers
    pub async fn shutdown(&self) {
        info!("TransactionManager shutting down gracefully");

        // Step 1: Stop the message processing loop FIRST
        // This prevents new messages from being processed
        {
            let mut running = self.running.lock().await;
            *running = false;
        }

        // Send shutdown signal to all spawned tasks
        if let Err(e) = self.shutdown_tx.send(()) {
            tracing::debug!("Failed to send shutdown signal (no receivers): {e}");
        }

        debug!("Message processing loop signaled to stop");
        
        // Step 2: Transport should already be closed by this point via events
        // But ensure it's closed just in case
        if let Err(e) = self.transport.close().await {
            debug!("Transport close during shutdown: {}", e);
        }
        
        // Step 3: Small drain period for in-flight messages
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        
        // Step 4: Wait for all transactions to reach Destroyed lifecycle state
        let client_count = self.client_transactions.lock().await.len();
        let server_count = self.server_transactions.lock().await.len();
        if client_count > 0 || server_count > 0 {
            debug!("Waiting for {} client and {} server transactions to reach Destroyed state", client_count, server_count);
            
            // Give transactions time to process their lifecycle transitions
            let mut wait_iterations = 0;
            loop {
                // Check if all transactions have reached Destroyed state
                let mut all_destroyed = true;
                
                {
                    let client_txs = self.client_transactions.lock().await;
                    for tx in client_txs.values() {
                        if tx.data().get_lifecycle() != TransactionLifecycle::Destroyed {
                            all_destroyed = false;
                            break;
                        }
                    }
                }
                
                if all_destroyed {
                    let server_txs = self.server_transactions.lock().await;
                    for tx in server_txs.values() {
                        if tx.data().get_lifecycle() != TransactionLifecycle::Destroyed {
                            all_destroyed = false;
                            break;
                        }
                    }
                }
                
                if all_destroyed {
                    debug!("All transactions reached Destroyed state");
                    break;
                }
                
                wait_iterations += 1;
                if wait_iterations > 20 { // 2 second timeout
                    warn!("Timeout waiting for transactions to reach Destroyed state, forcing cleanup");
                    break;
                }
                
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        
        // Now clear the transaction maps
        self.client_transactions.lock().await.clear();
        self.server_transactions.lock().await.clear();
        self.transaction_destinations.lock().await.clear();
        
        // Step 5: Emit TransactionEvent::ShutdownComplete
        // Broadcast to all event subscribers
        Self::broadcast_event(
            TransactionEvent::ShutdownComplete,
            &self.events_tx,
            &self.event_subscribers,
            Some(&self.transaction_to_subscribers),
            Some(self.clone()),
        ).await;
        
        // Step 5: Clear event subscribers
        self.event_subscribers.lock().await.clear();
        self.subscriber_to_transactions.lock().await.clear();
        self.transaction_to_subscribers.lock().await.clear();
        
        info!("TransactionManager shutdown complete - BOTTOM-UP");
    }

    /// Broadcasts a transaction event to all subscribers.
    ///
    /// This method is responsible for delivering transaction events to subscribers.
    /// It implements event filtering based on subscriber preferences, ensuring
    /// subscribers only receive events they're interested in.
    ///
    /// # Arguments
    /// * `event` - The transaction event to broadcast
    /// * `primary_tx` - The primary event channel
    /// * `subscribers` - Additional event subscribers
    /// * `transaction_to_subscribers` - Maps transactions to interested subscribers
    /// * `manager` - Optional manager for processing termination events
    pub(crate) async fn broadcast_event(
        event: TransactionEvent,
        primary_tx: &mpsc::Sender<TransactionEvent>,
        subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
        transaction_to_subscribers: Option<&Arc<Mutex<HashMap<TransactionKey, Vec<usize>>>>>,
        manager: Option<TransactionManager>,
    ) {
        // Extract transaction ID from the event for filtering
        let transaction_id = match &event {
            TransactionEvent::StateChanged { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::SuccessResponse { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::FailureResponse { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::ProvisionalResponse { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::TransactionTerminated { transaction_id } => Some(transaction_id),
            TransactionEvent::TimerTriggered { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::AckReceived { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::CancelReceived { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::InviteRequest { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::NonInviteRequest { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::AckRequest { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::CancelRequest { transaction_id, .. } => Some(transaction_id),
            // These events don't have a specific transaction ID
            TransactionEvent::StrayResponse { .. } => None,
            TransactionEvent::StrayAck { .. } => None,
            TransactionEvent::StrayCancel { .. } => None,
            TransactionEvent::StrayAckRequest { .. } => None,
            // Add other event types for completeness
            _ => None,
        };

        // Get list of interested subscribers for this transaction
        let interested_subscribers = if let (Some(tx_id), Some(tx_to_subs_map)) = (transaction_id, transaction_to_subscribers) {
            let tx_to_subs = tx_to_subs_map.lock().await;
            tx_to_subs.get(tx_id).cloned().unwrap_or_default()
        } else {
            Vec::new() // No specific subscribers for this transaction or global event
        };
        
        // Send to primary channel if it's available
        if let Err(e) = primary_tx.send(event.clone()).await {
            // During shutdown, channel closed errors are expected
            if e.to_string().contains("channel closed") {
                debug!("Primary event channel closed during shutdown (expected)");
            } else {
                warn!("Failed to send event to primary channel: {}", e);
            }
        }
        
        // Send to interested subscribers only
        let mut subs = subscribers.lock().await;
        
        // If we have transaction-specific subscribers, filter events
        if let Some(tx_to_subs_map) = transaction_to_subscribers {
            for (idx, sub) in subs.iter().enumerate() {
                // Send to this subscriber if:
                // 1. It's a global event (no transaction ID)
                // 2. This subscriber is interested in this transaction
                // 3. There are no interested subscribers specified (backward compatibility)
                let should_send = transaction_id.is_none() || 
                                 interested_subscribers.contains(&idx) ||
                                 interested_subscribers.is_empty();
                
                if should_send {
                    if let Err(e) = sub.send(event.clone()).await {
                        // During shutdown, channel closed errors are expected - use debug level
                        // Check if this is a channel closed error during shutdown
                        if e.to_string().contains("channel closed") {
                            debug!("Subscriber {} channel closed during shutdown (expected)", idx);
                        } else {
                            warn!("Failed to send event to subscriber {}: {}", idx, e);
                        }
                    }
                }
            }
        } else {
            // No transaction filtering, send to all (backward compatibility)
            for (idx, sub) in subs.iter().enumerate() {
                if let Err(e) = sub.send(event.clone()).await {
                    // During shutdown, channel closed errors are expected - use debug level
                    if e.to_string().contains("channel closed") {
                        debug!("Subscriber {} channel closed during shutdown (expected)", idx);
                    } else {
                        warn!("Failed to send event to subscriber {}: {}", idx, e);
                    }
                }
            }
        }
        
        // Special handling for transaction termination
        if let TransactionEvent::TransactionTerminated { transaction_id } = &event {
            if let Some(manager_instance) = manager {
                // Process the termination in a separate task to avoid deadlocks
                let tx_id = transaction_id.clone();
                let manager_clone = manager_instance.clone();
                tokio::spawn(async move {
                    manager_clone.process_transaction_terminated(&tx_id).await;
                });
            }
        }
    }

    /// Handle transaction termination event and clean up terminated transactions
    /// Uses lifecycle-based removal instead of immediate cleanup
    async fn process_transaction_terminated(&self, transaction_id: &TransactionKey) {
        debug!(%transaction_id, "Processing transaction termination - monitoring lifecycle for cleanup");
        
        // Start monitoring lifecycle state for proper cleanup timing
        let manager = self.clone();
        let tx_id = transaction_id.clone();
        
        tokio::spawn(async move {
            // Poll lifecycle state until Destroyed
            let mut cleanup_attempts = 0;
            loop {
                // Check if transaction is ready for cleanup
                let should_cleanup = {
                    // Try both client and server transactions
                    let client_txs = manager.client_transactions.lock().await;
                    let server_txs = manager.server_transactions.lock().await;
                    
                    let client_ready = client_txs.get(&tx_id)
                        .map(|tx| tx.data().get_lifecycle() == TransactionLifecycle::Destroyed)
                        .unwrap_or(false);
                    let server_ready = server_txs.get(&tx_id)
                        .map(|tx| tx.data().get_lifecycle() == TransactionLifecycle::Destroyed) 
                        .unwrap_or(false);
                    
                    client_ready || server_ready
                };
                
                if should_cleanup {
                    debug!(%tx_id, "Transaction lifecycle is Destroyed, performing cleanup");
                    manager.remove_terminated_transaction(&tx_id).await;
                    break;
                } 
                
                cleanup_attempts += 1;
                if cleanup_attempts > 50 { // 5 second timeout
                    warn!(%tx_id, "Lifecycle cleanup timeout, forcing removal");
                    manager.remove_terminated_transaction(&tx_id).await;
                    break;
                }
                
                // Check every 100ms
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        });
    }
    
    /// Actually remove a terminated transaction from all maps
    async fn remove_terminated_transaction(&self, transaction_id: &TransactionKey) {
        debug!(%transaction_id, "Removing terminated transaction after grace period");
        
        let mut terminated = false;
        
        // Try to remove from client transactions
        {
            let mut client_txs = self.client_transactions.lock().await;
            if let Some(tx) = client_txs.remove(transaction_id) {
                debug!(%transaction_id, "Removed terminated client transaction");
                terminated = true;
            }
        }
        
        // Try to remove from server transactions regardless of whether it was found in client transactions
        // This is a defensive approach in case the transaction was somehow duplicated
        {
            let mut server_txs = self.server_transactions.lock().await;
            if let Some(tx) = server_txs.remove(transaction_id) {
                debug!(%transaction_id, "Removed terminated server transaction");
                terminated = true;
            }
        }
        
        // Always remove from destinations map if present
        {
            let mut destinations = self.transaction_destinations.lock().await;
            if destinations.remove(transaction_id).is_some() {
                debug!(%transaction_id, "Removed transaction from destinations map");
            }
        }
        
        // **CRITICAL FIX**: Clean up subscriber mappings to prevent memory leak
        {
            let mut tx_to_subs = self.transaction_to_subscribers.lock().await;
            if let Some(subscriber_ids) = tx_to_subs.remove(transaction_id) {
                debug!(%transaction_id, subscriber_count = subscriber_ids.len(), "Removed transaction from subscriber mappings");
                
                // Also clean up reverse mappings
                drop(tx_to_subs); // Release lock before acquiring another
                let mut sub_to_txs = self.subscriber_to_transactions.lock().await;
                
                for subscriber_id in subscriber_ids {
                    if let Some(tx_list) = sub_to_txs.get_mut(&subscriber_id) {
                        tx_list.retain(|tx_id| tx_id != transaction_id);
                        
                        // If subscriber has no more transactions, remove it entirely
                        if tx_list.is_empty() {
                            sub_to_txs.remove(&subscriber_id);
                            debug!(%transaction_id, subscriber_id, "Removed empty subscriber mapping");
                        }
                    }
                }
            }
        }
        
        // Unregister from timer manager (defensive - it should auto-unregister)
        self.timer_manager.unregister_transaction(transaction_id).await;
        debug!(%transaction_id, "Unregistered transaction from timer manager");
        
        if terminated {
            debug!(%transaction_id, "Successfully cleaned up terminated transaction");
        } else {
            warn!(%transaction_id, "Transaction not found for termination - may have been already removed");
        }
        
        // Run cleanup to catch any other terminated transactions
        // This is a defensive measure to prevent resource leaks
        // We use spawn to avoid blocking and make this a background task
        let manager_clone = self.clone();
        tokio::spawn(async move {
            match manager_clone.cleanup_terminated_transactions().await {
                Ok(count) if count > 0 => {
                    debug!("Cleaned up {} additional terminated transactions", count);
                },
                Err(e) => {
                    error!("Error in background cleanup of terminated transactions: {}", e);
                },
                _ => {}
            }
        });
    }

    /// Start the message processing loop for handling incoming transport events
    pub(crate) fn start_message_loop(&self) {
        let transport_arc = self.transport.clone();
        let client_transactions = self.client_transactions.clone();
        let server_transactions = self.server_transactions.clone();
        let events_tx = self.events_tx.clone();
        let transport_rx = self.transport_rx.clone();
        let event_subscribers = self.event_subscribers.clone();
        let running = self.running.clone();
        let manager_arc = self.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            debug!("Starting transaction message loop");
            
            // Create a separate channel to receive events from transactions
            let (internal_tx, mut internal_rx) = mpsc::channel(100);
            
            // Set running flag
            let mut running_guard = running.lock().await;
            *running_guard = true;
            drop(running_guard);
            
            // Get the transport receiver
            let mut receiver = transport_rx.lock().await;
            
            // Run the message processing loop
            loop {
                // Check if we should continue running
                let running_guard = running.lock().await;
                let is_running = *running_guard;
                drop(running_guard);
                
                if !is_running {
                    debug!("Transaction manager stopping message loop");
                    break;
                }
                
                // Use tokio::select to wait for a message from either the transport or internal channel
                tokio::select! {
                    Some(message_event) = receiver.recv() => {
                        // Check if we're still running before processing
                        let still_running = *manager_arc.running.lock().await;
                        if still_running {
                            if let Err(e) = manager_arc.handle_transport_event(message_event).await {
                                error!("Error handling transport message: {}", e);
                            }
                        } else {
                            debug!("Skipping transport event processing - shutting down");
                        }
                    }
                    Some(transaction_event) = internal_rx.recv() => {
                        // Handle transaction events, particularly termination events
                        Self::broadcast_event(
                            transaction_event,
                            &events_tx,
                            &event_subscribers,
                            Some(&manager_arc.transaction_to_subscribers),
                            Some(manager_arc.clone()),
                        ).await;
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Transaction message loop received shutdown signal");
                        break;
                    }
                    else => {
                        // Both channels have been closed; exit loop
                        debug!("All message channels closed, exiting transaction message loop");
                        break;
                    }
                }
            }
            
            debug!("Transaction message loop exited");
        });
    }

    /// Helper function to get timer settings for a request
    pub(crate) fn timer_settings_for_request(&self, request: &Request) -> Option<TimerSettings> {
        // In the future, we could customize timer settings based on request properties
        // For now, just return a clone of the default settings
        Some(self.timer_settings.clone())
    }

    /// Forward a raw SIP request directly to a destination address.
    ///
    /// Used by the proxy forwarding path (Task 4.2) to relay an already-built
    /// request without creating a transaction — the response relay is handled
    /// externally.  The message is sent via the same transport that last
    /// received traffic from `destination`, falling back to UDP.
    pub async fn forward_request(&self, request: Request, destination: SocketAddr) -> Result<()> {
        debug!(%destination, "Forwarding SIP request via transport");
        // Prefer the transport manager (WS-aware) when available; otherwise
        // fall back to the primary transport.
        if let Some(ref tm) = self.transport_manager {
            tm.send_message(rvoip_sip_core::Message::Request(request), destination)
                .await
                .map_err(|e| Error::Transport(format!("Failed to forward request: {}", e)))
        } else {
            self.transport
                .send_message(rvoip_sip_core::Message::Request(request), destination)
                .await
                .map_err(|e| Error::Transport(format!("Failed to forward request: {}", e)))
        }
    }
}
