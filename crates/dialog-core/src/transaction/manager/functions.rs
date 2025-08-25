use std::net::SocketAddr;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, mpsc};
use tokio::time::timeout;
use tracing::{debug, error, warn, trace};

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;

use crate::transaction::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionAsync,
    TransactionKey, TransactionState, TransactionKind, TransactionEvent, InternalTransactionCommand
};
use crate::transaction::client::ClientTransaction;
use crate::transaction::client::TransactionExt as ClientTransactionExt;
use crate::transaction::server::ServerTransaction;
use crate::transaction::server::TransactionExt as ServerTransactionExt;

use super::TransactionManager;

impl TransactionManager {
    /// Retrieves the original request from a transaction.
    /// 
    /// In SIP protocol, each transaction begins with a request. According to RFC 3261, the transaction
    /// layer must store this request for potential retransmission and matching purposes. This method
    /// retrieves that original request from either a client or server transaction.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - For client transactions: Retrieve the request for retransmission (Timer A)
    /// - For server transactions: Access the request to create appropriate responses
    /// - For INVITE transactions: Create ACK requests for non-2xx responses
    /// - For CANCEL creation: Base the CANCEL request on the original INVITE
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1: Client Transaction state machines store original request
    /// - RFC 3261 Section 17.2.1: Server Transaction receives request
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID
    ///
    /// # Returns
    /// * `Result<Option<Request>>` - The original request, or None if not available
    pub async fn original_request(&self, tx_id: &TransactionKey) -> Result<Option<Request>> {
        // Try client transactions first
        {
            let client_txs = self.client_transactions.lock().await;
            if let Some(tx) = client_txs.get(tx_id) {
                if let Some(client_tx) = tx.as_client_transaction() {
                    return Ok(client_tx.original_request().await);
                }
            }
        }
        
        // Try server transactions
        {
            let server_txs = self.server_transactions.lock().await;
            if let Some(tx) = server_txs.get(tx_id) {
                if let Some(server_tx) = tx.as_server_transaction() {
                    return Ok(server_tx.original_request().await);
                }
            }
        }
        
        // Transaction not found
        Err(Error::transaction_not_found(tx_id.clone(), "original_request - transaction not found"))
    }

    /// Retrieves the last response from a transaction.
    ///
    /// In SIP, transactions track the last response they've sent or received. This is important
    /// for state machine operation, retransmission handling, and ACK generation.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - For client transactions: Access received responses for user notification
    /// - For server transactions: Retransmit last response if request retransmitted (RFC 3261 Section 17.2.1)
    /// - For INVITE transactions: Generate ACK requests based on final responses
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1.2: Client Transaction response handling
    /// - RFC 3261 Section 17.2.1: Server Transaction response retransmission
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID
    ///
    /// # Returns
    /// * `Result<Option<Response>>` - The last response, or None if not available
    pub async fn last_response(&self, tx_id: &TransactionKey) -> Result<Option<Response>> {
        // Try client transactions first
        {
            let client_txs = self.client_transactions.lock().await;
            if let Some(tx) = client_txs.get(tx_id) {
                if let Some(client_tx) = tx.as_client_transaction() {
                    return Ok(client_tx.last_response().await);
                }
            }
        }
        
        // Try server transactions
        {
            let server_txs = self.server_transactions.lock().await;
            if let Some(tx) = server_txs.get(tx_id) {
                if let Some(server_tx) = tx.as_server_transaction() {
                    // Use the ServerTransaction trait explicitly to avoid ambiguity
                    return Ok(ServerTransaction::last_response(server_tx));
                }
            }
        }
        
        // Transaction not found
        Err(Error::transaction_not_found(tx_id.clone(), "last_response - transaction not found"))
    }

    /// Retrieves the remote address of a transaction.
    ///
    /// The transaction layer must maintain the destination address for client transactions
    /// and the source address for server transactions, as dictated by RFC 3261.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - For client transactions: Destination for sending requests and receiving responses
    /// - For server transactions: Source for receiving requests and sending responses
    /// - For CANCEL: Determine the destination for CANCEL requests
    /// - For ACK: Determine the destination for ACK requests
    ///
    /// ## RFC References
    /// - RFC 3261 Section 18.1.1: SIP entities must route responses to client requests
    /// - RFC 3261 Section 18.2.2: Responses must be sent to address in top Via header
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID
    ///
    /// # Returns
    /// * `Result<SocketAddr>` - The remote address
    pub async fn remote_addr(&self, tx_id: &TransactionKey) -> Result<SocketAddr> {
        // Try client transactions first
        {
            let client_txs = self.client_transactions.lock().await;
            if let Some(tx) = client_txs.get(tx_id) {
                return Ok(tx.remote_addr());
            }
        }
        
        // Try server transactions
        {
            let server_txs = self.server_transactions.lock().await;
            if let Some(tx) = server_txs.get(tx_id) {
                return Ok(tx.remote_addr());
            }
        }
        
        // Transaction not found
        Err(Error::transaction_not_found(tx_id.clone(), "remote_addr - transaction not found"))
    }

    /// Wait for a transaction to reach a specific state.
    ///
    /// SIP transactions progress through well-defined state machines as described in RFC 3261.
    /// This function allows waiting for a transaction to reach a target state, which is useful
    /// for synchronizing application logic with transaction progress.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - Wait for client transactions to reach Completed state (response received)
    /// - Wait for server transactions to reach Terminated state before cleanup
    /// - Coordinate application logic with transaction state
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1: INVITE client transaction state machine
    /// - RFC 3261 Section 17.1.2: Non-INVITE client transaction state machine
    /// - RFC 3261 Section 17.2.1: INVITE server transaction state machine
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction state machine
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID
    /// * `target_state` - The state to wait for
    /// * `timeout_duration` - Maximum time to wait
    ///
    /// # Returns
    /// * `Result<bool>` - True if the state was reached, false if timed out
    ///
    /// # Example
    /// ```no_run
    /// # use std::time::Duration;
    /// # use rvoip_dialog_core::transaction::{TransactionManager, TransactionState, TransactionKey};
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) -> Result<(), Box<dyn std::error::Error>> {
    /// // Wait for the transaction to reach the Completed state
    /// let success = manager.wait_for_transaction_state(
    ///     tx_id,
    ///     TransactionState::Completed,
    ///     Duration::from_secs(5),
    /// ).await?;
    /// 
    /// if success {
    ///     println!("Transaction reached Completed state");
    /// } else {
    ///     println!("Timed out waiting for Completed state");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_transaction_state(
        &self,
        tx_id: &TransactionKey,
        target_state: TransactionState,
        timeout_duration: Duration,
    ) -> Result<bool> {
        // Subscribe to transaction events
        let mut rx = self.subscribe();
        
        debug!(%tx_id, ?target_state, "Waiting for transaction state");
        
        // Check if the transaction is already in the target state
        let current_state = match self.transaction_state(tx_id).await {
            Ok(state) => state,
            Err(e) => {
                warn!(%tx_id, error=%e, "Error checking transaction state");
                return Err(e);
            }
        };
        
        if current_state == target_state {
            debug!(%tx_id, ?current_state, "Transaction already in target state");
            return Ok(true);
        }
        
        // Start a timeout
        let start_time = std::time::Instant::now();
        
        // Wait for state change events with polling fallback
        loop {
            // Check if we've exceeded the timeout
            let elapsed = start_time.elapsed();
            if elapsed >= timeout_duration {
                debug!(%tx_id, ?current_state, ?target_state, elapsed=?elapsed, "Timeout waiting for state");
                return Ok(false);
            }
            
            // Calculate remaining time for this iteration
            let remaining = timeout_duration.saturating_sub(elapsed);
            let poll_interval = Duration::from_millis(50);
            let wait_time = std::cmp::min(remaining, poll_interval);
            
            // Check transaction state again to catch state changes that might have occurred
            // without receiving an event
            match self.transaction_state(tx_id).await {
                Ok(state) if state == target_state => {
                    debug!(%tx_id, ?state, "Transaction reached target state (detected by polling)");
                    return Ok(true);
                },
                Ok(_) => {}, // Not in target state yet, continue waiting
                Err(e) => {
                    // If transaction is not found, return false (it may have been terminated)
                    if matches!(e, Error::TransactionNotFound { .. }) {
                        debug!(%tx_id, "Transaction not found while waiting for state change, likely terminated");
                        return Ok(false);
                    }
                    warn!(%tx_id, error=%e, "Error checking transaction state");
                }
            }
            
            // Wait for an event or a timeout
            match tokio::time::timeout(wait_time, rx.recv()).await {
                // Got an event
                Ok(Some(TransactionEvent::StateChanged { 
                    transaction_id, 
                    new_state, 
                    ..
                })) if transaction_id == *tx_id && new_state == target_state => {
                    debug!(%tx_id, ?new_state, "Transaction reached target state (from event)");
                    return Ok(true);
                },
                // Transaction terminated, will never reach target state
                Ok(Some(TransactionEvent::TransactionTerminated { 
                    transaction_id, 
                    ..
                })) if transaction_id == *tx_id => {
                    debug!(%tx_id, "Transaction terminated while waiting for state change");
                    return Ok(false);
                },
                // Any other event or no event yet
                Ok(Some(_)) | Ok(None) | Err(_) => {
                    // Continue the loop, polling the transaction state again
                    trace!(%tx_id, ?target_state, elapsed=?elapsed, "Still waiting for state change");
                }
            }
        }
    }

    /// Wait for a transaction to receive a final response.
    ///
    /// In SIP, final responses have status codes ≥ 200. This method waits until a transaction
    /// receives a final response or times out, simplifying application flow control.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - UAC waiting for call setup completion
    /// - Error handling for failed requests
    /// - Dialog creation after 2xx responses
    ///
    /// ## RFC References
    /// - RFC 3261 Section 8.1.3.3: Response codes
    /// - RFC 3261 Section 17.1.1.2: INVITE client transaction receiving responses
    /// - RFC 3261 Section 17.1.2.2: Non-INVITE client transaction receiving responses
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID
    /// * `timeout_duration` - Maximum time to wait
    ///
    /// # Returns
    /// * `Result<Option<Response>>` - The final response if received, None if timed out
    ///
    /// # Example
    /// ```no_run
    /// # use std::time::Duration;
    /// # use rvoip_dialog_core::transaction::{TransactionManager, TransactionKey};
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) -> Result<(), Box<dyn std::error::Error>> {
    /// // Wait for a final response (2xx-6xx)
    /// let response = manager.wait_for_final_response(
    ///     tx_id,
    ///     Duration::from_secs(5),
    /// ).await?;
    /// 
    /// match response {
    ///     Some(resp) => println!("Received final response: {}", resp.status()),
    ///     None => println!("Timed out waiting for final response"),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_final_response(
        &self,
        tx_id: &TransactionKey,
        timeout_duration: Duration,
    ) -> Result<Option<Response>> {
        // Subscribe to transaction events
        let mut rx = self.subscribe();
        
        debug!(%tx_id, "Waiting for final response");
        
        // Check if the transaction already has a final response
        match self.last_response(tx_id).await {
            Ok(Some(response)) if response.status().as_u16() >= 200 => {
                debug!(%tx_id, status=%response.status(), "Transaction already has final response");
                return Ok(Some(response));
            },
            Ok(_) => {}, // No response or non-final response
            Err(e) => {
                warn!(%tx_id, error=%e, "Error checking transaction last response");
                return Err(e);
            }
        };
        
        // Start a timeout
        let start_time = std::time::Instant::now();
        
        // Poll periodically for a response
        loop {
            // Check if we've exceeded the timeout
            let elapsed = start_time.elapsed();
            if elapsed >= timeout_duration {
                debug!(%tx_id, elapsed=?elapsed, "Timeout waiting for final response");
                return Ok(None);
            }
            
            // Calculate remaining time for this iteration
            let remaining = timeout_duration.saturating_sub(elapsed);
            let poll_interval = Duration::from_millis(100);
            let wait_time = std::cmp::min(remaining, poll_interval);
            
            // Poll for a final response
            match self.last_response(tx_id).await {
                Ok(Some(response)) if response.status().as_u16() >= 200 => {
                    debug!(%tx_id, status=%response.status(), "Received final response (detected by polling)");
                    return Ok(Some(response));
                },
                Ok(_) => {}, // No final response yet
                Err(e) => {
                    // If transaction is gone, we'll never get a response
                    if matches!(e, Error::TransactionNotFound { .. }) {
                        debug!(%tx_id, "Transaction not found while waiting for final response");
                        return Ok(None);
                    }
                    warn!(%tx_id, error=%e, "Error checking transaction last response");
                }
            }
            
            // Wait for events with timeout
            match tokio::time::timeout(wait_time, rx.recv()).await {
                // Received a success response event
                Ok(Some(TransactionEvent::SuccessResponse { 
                    transaction_id, 
                    response, 
                    ..
                })) if transaction_id == *tx_id => {
                    debug!(%tx_id, status=%response.status(), "Received success response event");
                    return Ok(Some(response));
                },
                // Received a failure response event
                Ok(Some(TransactionEvent::FailureResponse { 
                    transaction_id, 
                    response, 
                    ..
                })) if transaction_id == *tx_id => {
                    debug!(%tx_id, status=%response.status(), "Received failure response event");
                    return Ok(Some(response));
                },
                // Transaction terminated with possible final response
                Ok(Some(TransactionEvent::TransactionTerminated { 
                    transaction_id, 
                    ..
                })) if transaction_id == *tx_id => {
                    debug!(%tx_id, "Transaction terminated, checking for final response before returning");
                    
                    // Last attempt to get a final response
                    match self.last_response(tx_id).await {
                        Ok(Some(response)) if response.status().as_u16() >= 200 => {
                            debug!(%tx_id, status=%response.status(), "Found final response after termination");
                            return Ok(Some(response));
                        },
                        _ => {
                            debug!(%tx_id, "No final response after termination");
                            return Ok(None);
                        }
                    }
                },
                // Any other event or no event yet
                Ok(Some(_)) | Ok(None) | Err(_) => {
                    // Continue the loop, will poll again
                    trace!(%tx_id, elapsed=?elapsed, "Still waiting for final response");
                }
            }
        }
    }

    /// Get the total number of active transactions.
    ///
    /// Monitors the count of active transactions. This is useful for diagnostics, 
    /// load monitoring, and ensuring proper cleanup.
    ///
    /// ## Uses in SIP Transaction Layer
    ///
    /// - Monitoring transaction count for resource utilization
    /// - Detecting transaction leaks
    /// - Load balancing in high-volume systems
    ///
    /// # Returns
    /// * `usize` - The number of active transactions
    pub async fn transaction_count(&self) -> usize {
        let client_count = self.client_transactions.lock().await.len();
        let server_count = self.server_transactions.lock().await.len();
        client_count + server_count
    }

    /// Terminates a transaction.
    ///
    /// Forces a transaction to terminate regardless of its current state.
    /// RFC 3261 defines normal termination conditions for each transaction type,
    /// but sometimes external factors require immediate termination.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - Force termination of stalled transactions
    /// - Clean up during application shutdown
    /// - Release resources for canceled operations
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1.2: Normal INVITE client transaction termination
    /// - RFC 3261 Section 17.1.2.2: Normal non-INVITE client transaction termination
    /// - RFC 3261 Section 17.2.1: Normal INVITE server transaction termination
    /// - RFC 3261 Section 17.2.2: Normal non-INVITE server transaction termination
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID
    ///
    /// # Returns
    /// * `Result<()>` - Success or an error if the transaction doesn't exist
    pub async fn terminate_transaction(&self, tx_id: &TransactionKey) -> Result<()> {
        let mut terminated = false;
        
        // Try client transactions first
        {
            let mut client_txs = self.client_transactions.lock().await;
            if let Some(tx) = client_txs.remove(tx_id) {
                terminated = true;
            }
        }
        
        // If not found in client transactions, try server transactions
        if !terminated {
            let mut server_txs = self.server_transactions.lock().await;
            if let Some(tx) = server_txs.remove(tx_id) {
                terminated = true;
            }
        }
        
        // Also remove from destinations map if it's there
        {
            let mut dest_map = self.transaction_destinations.lock().await;
            dest_map.remove(tx_id);
        }
        
        // **CRITICAL FIX**: Clean up subscriber mappings to prevent memory leak
        {
            let mut tx_to_subs = self.transaction_to_subscribers.lock().await;
            if let Some(subscriber_ids) = tx_to_subs.remove(tx_id) {
                debug!(%tx_id, subscriber_count = subscriber_ids.len(), "Removed terminated transaction from subscriber mappings");
                
                // Also clean up reverse mappings
                drop(tx_to_subs); // Release lock before acquiring another
                let mut sub_to_txs = self.subscriber_to_transactions.lock().await;
                
                for subscriber_id in subscriber_ids {
                    if let Some(tx_list) = sub_to_txs.get_mut(&subscriber_id) {
                        tx_list.retain(|id| id != tx_id);
                        
                        // If subscriber has no more transactions, remove it entirely
                        if tx_list.is_empty() {
                            sub_to_txs.remove(&subscriber_id);
                            debug!(%tx_id, subscriber_id, "Removed empty subscriber mapping");
                        }
                    }
                }
            }
        }
        
        // Unregister from timer manager
        self.timer_manager.unregister_transaction(tx_id).await;
        debug!(%tx_id, "Unregistered transaction from timer manager");
        
        if terminated {
            // Broadcast a transaction terminated event
            let event = TransactionEvent::TransactionTerminated { 
                transaction_id: tx_id.clone() 
            };
            
            // Use the broadcast_event utility
            Self::broadcast_event(
                event,
                &self.events_tx,
                &self.event_subscribers,
                Some(&self.transaction_to_subscribers),
                None,
            ).await;
            
            Ok(())
        } else {
            Err(Error::transaction_not_found(tx_id.clone(), "terminate_transaction - transaction not found"))
        }
    }

    /// Cleanup terminated transactions.
    ///
    /// Removes terminated transactions to free up resources. According to RFC 3261,
    /// transactions should transition to the Terminated state before being removed
    /// from the transaction set.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - Regular housekeeping of transaction tables
    /// - Resource management in high-volume systems
    /// - Final cleanup as required by RFC 3261 Section 17
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1.2: INVITE client transaction terminated state
    /// - RFC 3261 Section 17.1.2.2: Non-INVITE client transaction terminated state
    /// - RFC 3261 Section 17.2.1: INVITE server transaction terminated state 
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction terminated state
    ///
    /// # Returns
    /// * `Result<usize>` - The number of transactions cleaned up
    pub async fn cleanup_terminated_transactions(&self) -> Result<usize> {
        let mut cleaned_count = 0;
        let mut terminated_transaction_ids = Vec::new();
        
        // Cleanup client transactions
        {
            let mut client_txs = self.client_transactions.lock().await;
            let terminated_keys: Vec<TransactionKey> = client_txs.iter()
                .filter(|(_, tx)| tx.state() == TransactionState::Terminated)
                .map(|(k, _)| k.clone())
                .collect();
            
            debug!("Found {} terminated client transactions", terminated_keys.len());
            for key in terminated_keys {
                debug!(%key, "Removing terminated client transaction");
                client_txs.remove(&key);
                terminated_transaction_ids.push(key);
                cleaned_count += 1;
            }
        }
        
        // Cleanup server transactions
        {
            let mut server_txs = self.server_transactions.lock().await;
            let terminated_keys: Vec<TransactionKey> = server_txs.iter()
                .filter(|(_, tx)| tx.state() == TransactionState::Terminated)
                .map(|(k, _)| k.clone())
                .collect();
            
            debug!("Found {} terminated server transactions", terminated_keys.len());
            for key in terminated_keys {
                debug!(%key, "Removing terminated server transaction");
                server_txs.remove(&key);
                terminated_transaction_ids.push(key);
                cleaned_count += 1;
            }
        }
        
        // Cleanup orphaned entries in the transaction_destinations map
        {
            let mut dest_map = self.transaction_destinations.lock().await;
            let client_txs = self.client_transactions.lock().await;
            let server_txs = self.server_transactions.lock().await;
            
            let orphaned_keys: Vec<TransactionKey> = dest_map.keys()
                .filter(|k| !client_txs.contains_key(k) && !server_txs.contains_key(k))
                .cloned()
                .collect();
            
            debug!("Found {} orphaned destination entries", orphaned_keys.len());
            for key in orphaned_keys {
                debug!(%key, "Removing orphaned destination entry");
                dest_map.remove(&key);
            }
        }
        
        // **CRITICAL FIX**: Clean up subscriber mappings for all terminated transactions
        {
            let mut tx_to_subs = self.transaction_to_subscribers.lock().await;
            let mut subscriber_ids_to_clean = Vec::new();
            
            for tx_id in &terminated_transaction_ids {
                if let Some(subscriber_ids) = tx_to_subs.remove(tx_id) {
                    debug!(%tx_id, subscriber_count = subscriber_ids.len(), "Removed terminated transaction from subscriber mappings");
                    subscriber_ids_to_clean.extend(subscriber_ids);
                }
            }
            
            drop(tx_to_subs); // Release lock before acquiring another
            
            // Clean up reverse mappings
            if !subscriber_ids_to_clean.is_empty() {
                let mut sub_to_txs = self.subscriber_to_transactions.lock().await;
                
                for subscriber_id in subscriber_ids_to_clean {
                    if let Some(tx_list) = sub_to_txs.get_mut(&subscriber_id) {
                        let original_len = tx_list.len();
                        tx_list.retain(|tx_id| !terminated_transaction_ids.contains(tx_id));
                        
                        // If subscriber has no more transactions, remove it entirely
                        if tx_list.is_empty() {
                            sub_to_txs.remove(&subscriber_id);
                            debug!(subscriber_id, "Removed empty subscriber mapping");
                        } else if tx_list.len() != original_len {
                            debug!(subscriber_id, old_count = original_len, new_count = tx_list.len(), "Cleaned up subscriber transaction list");
                        }
                    }
                }
            }
        }
        
        // Unregister terminated transactions from timer manager
        for tx_id in &terminated_transaction_ids {
            self.timer_manager.unregister_transaction(tx_id).await;
            debug!(%tx_id, "Unregistered terminated transaction from timer manager");
        }
        
        // Also manually check for client transactions that look terminated but don't have the state set
        {
            let mut client_txs = self.client_transactions.lock().await;
            // Look for transactions whose event_loop_handle is None or completed
            let potentially_terminated: Vec<TransactionKey> = client_txs.iter()
                .filter_map(|(k, tx)| {
                    // If we can downcast to ClientTransactionExt
                    if let Some(client_tx) = tx.as_client_transaction() {
                        // Check if handle is None or completed
                        let is_terminated = if tx.state() == TransactionState::Terminated {
                            true
                        } else {
                            // Also check event loop handle completion
                            false
                        };
                        if is_terminated {
                            Some(k.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();
                
            for key in potentially_terminated {
                debug!(%key, "Removing potentially terminated client transaction");
                client_txs.remove(&key);
                cleaned_count += 1;
            }
        }
        
        debug!("Cleaned up {} terminated transactions", cleaned_count);
        Ok(cleaned_count)
    }

    /// Find transactions related to the given transaction.
    ///
    /// Some SIP methods (like CANCEL and ACK) are related to other transactions.
    /// This function finds these related transactions to support operations 
    /// that span multiple transactions.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - Finding INVITE for a CANCEL transaction
    /// - Finding INVITE for an incoming ACK
    /// - Managing transaction relationships for dialog creation
    ///
    /// ## RFC References
    /// - RFC 3261 Section 9.1: CANCEL relationship to INVITE
    /// - RFC 3261 Section 17.1.1.3: ACK for non-2xx responses
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID
    ///
    /// # Returns
    /// * `Result<Vec<TransactionKey>>` - List of related transaction IDs
    pub async fn find_related_transactions(&self, tx_id: &TransactionKey) -> Result<Vec<TransactionKey>> {
        let mut related = Vec::new();
        
        // Get the original request from the transaction
        let request = match self.original_request(tx_id).await? {
            Some(req) => req,
            None => return Ok(Vec::new()), // No request, no related transactions
        };
        
        // For INVITE transactions, look for related CANCEL transactions
        if request.method() == Method::Invite {
            // Check client transactions for related CANCEL
            let client_txs = self.client_transactions.lock().await;
            let cancel_matches: Vec<TransactionKey> = client_txs.iter()
                .filter(|(k, _)| k.method() == &Method::Cancel && !k.is_server)
                .map(|(k, _)| k.clone())
                .collect();
            drop(client_txs);
            
            for cancel_key in cancel_matches {
                if let Ok(Some(cancel_req)) = self.original_request(&cancel_key).await {
                    // Check if the CANCEL matches this INVITE
                    if crate::transaction::method::cancel::is_cancel_for_invite(&cancel_req, &request) {
                        related.push(cancel_key);
                    }
                }
            }
        }
        
        // For CANCEL transactions, find the related INVITE
        if request.method() == Method::Cancel {
            if let Some(invite_key) = self.find_invite_transaction_for_cancel(&request).await? {
                related.push(invite_key);
            }
        }
        
        Ok(related)
    }

    /// Retry sending a request.
    ///
    /// Provides an application-initiated retransmission mechanism beyond the automatic
    /// retransmissions governed by transaction timers. This is useful for recovering
    /// from known network issues.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - Recovery from known network issues
    /// - Attempting to send a request through an alternate path
    /// - Application-controlled reliability enhancement
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1.2: INVITE client transaction retransmissions
    /// - RFC 3261 Section 17.1.2.2: Non-INVITE client transaction retransmissions
    ///
    /// # Arguments
    /// * `tx_id` - The transaction ID
    ///
    /// # Returns
    /// * `Result<()>` - Success or an error if retry isn't possible
    pub async fn retry_request(&self, tx_id: &TransactionKey) -> Result<()> {
        // Only client transactions can retry a request
        if tx_id.is_server() {
            return Err(Error::Other("Cannot retry a server transaction".to_string()));
        }
        
        // Get the client transaction
        let client_txs = self.client_transactions.lock().await;
        let tx = client_txs.get(tx_id)
            .ok_or_else(|| Error::transaction_not_found(tx_id.clone(), "retry_request - transaction not found"))?;
        
        // Get a ClientTransaction reference
        if let Some(client_tx) = tx.as_client_transaction() {
            // Get the original request
            let request = client_tx.original_request().await
                .ok_or_else(|| Error::Other("No original request available for retry".to_string()))?;
            
            // Get the destination
            let destination = client_tx.remote_addr();
            
            // Send the request directly via the transport
            let transport = self.transport.clone();
            transport.send_message(Message::Request(request), destination).await
                .map_err(|e| Error::transport_error(e, "Failed to retry request"))
        } else {
            Err(Error::Other("Failed to downcast to client transaction".to_string()))
        }
    }

    /// Process a request for an existing server transaction.
    ///
    /// This method allows direct processing of a request (like ACK or CANCEL) by a 
    /// specific server transaction. It's primarily used for handling ACK requests 
    /// for non-2xx responses in INVITE server transactions according to RFC 3261.
    ///
    /// ## Uses in SIP Transaction Layer
    /// 
    /// - Processing ACK requests for non-2xx responses
    /// - Processing retransmitted requests
    /// - Test environments that need direct access to transactions
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.2.1: INVITE server transaction ACK handling
    ///
    /// # Arguments
    /// * `tx_id` - The server transaction ID
    /// * `request` - The SIP request to process
    ///
    /// # Returns
    /// * `Result<()>` - Success or an error if the transaction doesn't exist or processing fails
    pub async fn process_request(&self, tx_id: &TransactionKey, request: Request) -> Result<()> {
        // Only server transactions can process requests
        if !tx_id.is_server() {
            return Err(Error::Other("Cannot process request for client transaction".to_string()));
        }
        
        // Get the server transaction
        let server_txs = self.server_transactions.lock().await;
        let tx = server_txs.get(tx_id)
            .ok_or_else(|| Error::transaction_not_found(tx_id.clone(), "process_request - transaction not found"))?;
        
        // Clone it so we can drop the lock before the async call
        let tx_clone = tx.clone();
        drop(server_txs);
        
        // Process the request using the transaction's implementation
        tx_clone.process_request(request).await
    }
} 