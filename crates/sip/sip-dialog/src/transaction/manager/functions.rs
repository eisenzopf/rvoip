use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rvoip_sip_core::prelude::*;
use tracing::debug;

use crate::diagnostics;
use crate::transaction::client::TransactionExt as ClientTransactionExt;
use crate::transaction::error::{Error, Result};
use crate::transaction::runner::HasLifecycle;
use crate::transaction::server::ServerTransaction;
use crate::transaction::server::TransactionExt as ServerTransactionExt;
use crate::transaction::{
    InternalTransactionCommand, TransactionKey, TransactionLifecycle, TransactionState,
};

use super::TransactionManager;

/// Exact transaction generation accepted by an explicit termination request.
///
/// Keeping the transaction `Arc` in the manager-owned job also keeps its
/// admission owner alive. A delayed cleanup job can therefore never terminate
/// a later transaction that happens to reuse the same SIP wire key.
enum ExplicitTerminationTarget {
    Client(super::ArcClientTransaction),
    Server(std::sync::Arc<dyn ServerTransaction>),
}

/// Cancellation-safe completion owned by the manager cleanup worker after
/// queue admission. The public waiter may be dropped at any point without
/// cancelling exact transaction cleanup.
pub(crate) struct ExplicitTerminationOperation {
    transaction_id: TransactionKey,
    target: ExplicitTerminationTarget,
    result: std::sync::Mutex<Option<Result<()>>>,
    completed: std::sync::atomic::AtomicBool,
    execution_claimed: std::sync::atomic::AtomicBool,
    notify: tokio::sync::Notify,
    manager_operation: std::sync::Mutex<Option<super::TransactionManagerAdmissionGuard>>,
}

struct ExplicitTerminationExecution {
    operation: std::sync::Arc<ExplicitTerminationOperation>,
}

impl Drop for ExplicitTerminationExecution {
    fn drop(&mut self) {
        self.operation.complete(Err(Error::Other(
            "explicit transaction termination worker stopped before completion".into(),
        )));
    }
}

impl ExplicitTerminationOperation {
    fn new(
        transaction_id: TransactionKey,
        target: ExplicitTerminationTarget,
        manager_operation: super::TransactionManagerAdmissionGuard,
    ) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            transaction_id,
            target,
            result: std::sync::Mutex::new(None),
            completed: std::sync::atomic::AtomicBool::new(false),
            execution_claimed: std::sync::atomic::AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
            manager_operation: std::sync::Mutex::new(Some(manager_operation)),
        })
    }

    pub(super) fn transaction_id(&self) -> &TransactionKey {
        &self.transaction_id
    }

    fn client_transaction(&self) -> Option<super::ArcClientTransaction> {
        match &self.target {
            ExplicitTerminationTarget::Client(transaction) => Some(transaction.clone()),
            ExplicitTerminationTarget::Server(_) => None,
        }
    }

    fn server_transaction(&self) -> Option<std::sync::Arc<dyn ServerTransaction>> {
        match &self.target {
            ExplicitTerminationTarget::Client(_) => None,
            ExplicitTerminationTarget::Server(transaction) => Some(transaction.clone()),
        }
    }

    fn begin_execution(self: &std::sync::Arc<Self>) -> Result<ExplicitTerminationExecution> {
        self.execution_claimed
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .map_err(|_| Error::Other("explicit transaction termination ran twice".into()))?;
        diagnostics::record_explicit_termination_in_flight(1);
        Ok(ExplicitTerminationExecution {
            operation: std::sync::Arc::clone(self),
        })
    }

    fn complete(&self, result: Result<()>) {
        if self
            .completed
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }
        if self
            .execution_claimed
            .load(std::sync::atomic::Ordering::Acquire)
        {
            diagnostics::record_explicit_termination_in_flight(-1);
        }
        diagnostics::record_explicit_termination_completed(result.is_ok());
        *self
            .result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
        // Release shutdown admission at manager completion, not when a slow
        // or abandoned public waiter eventually drops its Arc.
        self.manager_operation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.notify.notify_waiters();
    }

    pub(super) async fn execute(self: &std::sync::Arc<Self>, manager: &TransactionManager) {
        let execution = match self.begin_execution() {
            Ok(execution) => execution,
            // A duplicate queue key can share a cleanup batch, but it must
            // never steal completion from the exact execution already in
            // progress.
            Err(_) => return,
        };
        let result = manager.terminate_transaction_supervised(self).await;
        self.complete(result);
        drop(execution);
    }

    async fn wait(&self) -> Result<()> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(result) = self
                .result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                return result;
            }
            notified.await;
        }
    }
}

async fn wait_for_atomic_state(
    state: std::sync::Arc<crate::transaction::AtomicTransactionState>,
    target_state: TransactionState,
    timeout_duration: Duration,
) -> bool {
    let mut changes = state.subscribe();
    let wait = async {
        loop {
            let current = state.get();
            if current == target_state {
                return true;
            }
            if current == TransactionState::Terminated {
                return false;
            }
            if changes.changed().await.is_err() {
                return state.get() == target_state;
            }
        }
    };
    tokio::time::timeout(timeout_duration, wait)
        .await
        .unwrap_or(false)
}

impl TransactionManager {
    /// Clone the exact state authority retained by a compact Timer J/K
    /// tombstone. The returned `Arc` remains waitable after the deadline
    /// worker removes the tombstone from the public existence index.
    pub(super) fn compact_non_invite_state(
        &self,
        tx_id: &TransactionKey,
    ) -> Option<std::sync::Arc<crate::transaction::AtomicTransactionState>> {
        self.compact_non_invite_tombstones
            .get(tx_id)
            .map(|entry| match entry.value() {
                crate::transaction::lifecycle_scheduler::CompactNonInviteTombstone::Client {
                    state,
                    ..
                }
                | crate::transaction::lifecycle_scheduler::CompactNonInviteTombstone::Server {
                    state,
                    ..
                } => std::sync::Arc::clone(state),
            })
    }

    pub(super) fn request_transaction_runner_stop(&self, tx_id: &TransactionKey) {
        if let Some(tx) = self
            .client_transactions
            .get(tx_id)
            .map(|entry| entry.value().clone())
        {
            let data = tx.data();
            if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
                data.cmd_tx.try_send(InternalTransactionCommand::Terminate)
            {
                data.set_lifecycle(TransactionLifecycle::Destroyed);
            }
        }

        if let Some(tx) = self
            .server_transactions
            .get(tx_id)
            .map(|entry| entry.value().clone())
        {
            let data = tx.data();
            if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
                data.cmd_tx.try_send(InternalTransactionCommand::Terminate)
            {
                data.set_lifecycle(TransactionLifecycle::Destroyed);
            }
        }
    }

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
        // Extract Arc out of each shard before awaiting on per-tx state.
        let client_arc = self
            .client_transactions
            .get(tx_id)
            .map(|r| r.value().clone());
        if let Some(tx) = client_arc {
            if let Some(client_tx) = tx.as_client_transaction() {
                return Ok(client_tx.original_request().await);
            }
        }
        let server_arc = self
            .server_transactions
            .get(tx_id)
            .map(|r| r.value().clone());
        if let Some(tx) = server_arc {
            if let Some(server_tx) = tx.as_server_transaction() {
                return Ok(server_tx.original_request().await);
            }
        }
        if let Some(retired) = self.retired_client_original_request(tx_id)? {
            return Ok(Some(retired));
        }
        Err(Error::transaction_not_found(
            tx_id.clone(),
            "original_request - transaction not found",
        ))
    }

    /// Resolve the exact Request-URI retained for a 401/407 challenge.
    ///
    /// Non-INVITE runners may compact into Timer K state before dialog-core
    /// consumes their lossless event. The completion record preserves this
    /// one challenge-only value without retaining every parsed request.
    pub(crate) fn auth_challenge_request_uri(&self, tx_id: &TransactionKey) -> Option<Uri> {
        self.client_completion(tx_id)
            .and_then(|completion| completion.auth_challenge_request_uri())
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
        let client_arc = self
            .client_transactions
            .get(tx_id)
            .map(|r| r.value().clone());
        if let Some(tx) = client_arc {
            if let Some(client_tx) = tx.as_client_transaction() {
                return Ok(client_tx.last_response().await);
            }
        }
        let server_arc = self
            .server_transactions
            .get(tx_id)
            .map(|r| r.value().clone());
        if let Some(tx) = server_arc {
            if let Some(server_tx) = tx.as_server_transaction() {
                return Ok(ServerTransaction::last_response(&*server_tx));
            }
        }
        if let Some((wire, _)) = self
            .compact_non_invite_tombstones
            .get(tx_id)
            .and_then(|entry| {
                entry
                    .value()
                    .server_replay()
                    .map(|(wire, route)| (wire.clone(), route.clone()))
            })
        {
            return match rvoip_sip_core::parse_message(&wire)? {
                Message::Response(response) => Ok(Some(response)),
                Message::Request(_) => Err(Error::Other(
                    "compact Timer J response bytes parsed as a request".into(),
                )),
            };
        }
        if let Some(completion) = self.client_completion(tx_id) {
            return completion.last_response();
        }
        Err(Error::transaction_not_found(
            tx_id.clone(),
            "last_response - transaction not found",
        ))
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
        if let Some(entry) = self.client_transactions.get(tx_id) {
            return Ok(entry.value().remote_addr());
        }
        if let Some(entry) = self.server_transactions.get(tx_id) {
            return Ok(entry.value().remote_addr());
        }
        if let Some(entry) = self.compact_non_invite_tombstones.get(tx_id) {
            if let Some((_, route)) = entry.value().server_replay() {
                return Ok(route.destination);
            }
            if entry.value().is_client() {
                if let Some(destination) =
                    self.with_client_response_route_state(tx_id, |state| state.route().destination)
                {
                    return Ok(destination);
                }
            }
        }
        Err(Error::transaction_not_found(
            tx_id.clone(),
            "remote_addr - transaction not found",
        ))
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
    /// # use rvoip_sip_dialog::transaction::{TransactionManager, TransactionState, TransactionKey};
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
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), ?target_state, "Waiting for transaction state");
        // Capture a compact Timer J/K state authority first. If expiry removes
        // the map entry after this clone, the waiter still observes the exact
        // terminal write. A lookup beginning after removal correctly reports
        // TransactionNotFound instead of reviving an expired tombstone.
        if let Some(state) = self.compact_non_invite_state(tx_id) {
            return Ok(wait_for_atomic_state(state, target_state, timeout_duration).await);
        }
        if let Some(completion) = self.client_completion(tx_id) {
            return Ok(completion
                .wait_for_state(target_state, timeout_duration)
                .await);
        }
        let state = if let Some(transaction) = self
            .server_transactions
            .get(tx_id)
            .map(|entry| entry.value().clone())
        {
            transaction.data().state.clone()
        } else {
            return Err(Error::transaction_not_found(
                tx_id.clone(),
                "wait_for_transaction_state - transaction not found",
            ));
        };
        Ok(wait_for_atomic_state(state, target_state, timeout_duration).await)
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
    /// # use rvoip_sip_dialog::transaction::{TransactionManager, TransactionKey};
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
        debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Waiting for final response");
        Ok(
            match self
                .wait_for_client_transaction_outcome(tx_id, timeout_duration)
                .await?
            {
                Some(crate::transaction::ClientTransactionOutcome::FinalResponse(response)) => {
                    Some(response)
                }
                Some(crate::transaction::ClientTransactionOutcome::Failure(_)) | None => None,
            },
        )
    }

    /// Wait for the exact typed outcome of a client transaction without
    /// allocating a public event subscription.
    pub async fn wait_for_client_transaction_outcome(
        &self,
        tx_id: &TransactionKey,
        timeout_duration: Duration,
    ) -> Result<Option<crate::transaction::ClientTransactionOutcome>> {
        let completion = self.client_completion(tx_id).ok_or_else(|| {
            Error::transaction_not_found(
                tx_id.clone(),
                "wait_for_client_transaction_outcome - transaction not found",
            )
        })?;
        completion.wait_for_outcome(timeout_duration).await
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
        let client_count = self.client_transactions.len();
        let server_count = self.server_transactions.len();
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
        // Acquire the shutdown fence before resolving the exact generation.
        // The transaction Arc captured below owns its wire-key admission
        // generation until the manager worker completes the request.
        let manager_operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| {
                Error::Other(
                    "transaction manager is stopping; termination admission is closed".into(),
                )
            })?;

        // A compact Timer J/K generation has already terminated the active
        // runner and owns response-before-wait observability through its RFC
        // retention horizon. External termination is idempotent here.
        if self.compact_non_invite_tombstones.contains_key(tx_id) {
            return Ok(());
        }

        let target = if let Some(transaction) = self
            .client_transactions
            .get(tx_id)
            .map(|entry| entry.value().clone())
        {
            ExplicitTerminationTarget::Client(transaction)
        } else if let Some(transaction) = self
            .server_transactions
            .get(tx_id)
            .map(|entry| entry.value().clone())
        {
            ExplicitTerminationTarget::Server(transaction)
        } else {
            // Compact retirement can race the first lookup. Retired client
            // completion is likewise a successful, already-finished exact
            // generation rather than TransactionNotFound.
            if self.compact_non_invite_tombstones.contains_key(tx_id)
                || self.client_completion(tx_id).is_some()
            {
                return Ok(());
            }
            return Err(Error::transaction_not_found(
                tx_id.clone(),
                "terminate_transaction - transaction not found",
            ));
        };

        let operation = ExplicitTerminationOperation::new(tx_id.clone(), target, manager_operation);
        let cleanup_tx = self.terminated_cleanup_tx.as_ref().ok_or_else(|| {
            Error::Other("transaction manager cleanup worker is unavailable".into())
        })?;

        // Waiting for bounded queue capacity is cancellation-safe: no work is
        // published until the permit is acquired. Once `send` returns, the
        // manager queue owns the operation and its admission guard, so dropping
        // this public future cannot cancel protocol cleanup.
        let permit = cleanup_tx
            .reserve()
            .await
            .map_err(|_| Error::Other("transaction manager cleanup worker is closed".into()))?;
        self.explicit_termination_operations
            .entry(tx_id.clone())
            .or_default()
            .push(std::sync::Arc::clone(&operation));
        permit.send(tx_id.clone());
        diagnostics::record_explicit_termination_enqueued();
        operation.wait().await
    }

    async fn terminate_transaction_supervised(
        &self,
        operation: &ExplicitTerminationOperation,
    ) -> Result<()> {
        let tx_id = operation.transaction_id();
        // A compact Timer J/K generation has already terminated the active
        // runner and owns both retransmission absorption and the exact client
        // completion. External termination is therefore idempotent; shortening
        // its RFC deadline would lose response-before-wait observability.
        if self.compact_non_invite_tombstones.contains_key(tx_id) {
            return Ok(());
        }

        let client_transaction = operation.client_transaction();
        let server_transaction = operation.server_transaction();

        // Record the exact result before the runner can snapshot a compact
        // successor. The completion cell is first-writer-wins, so a response
        // or more-specific failure that arrived first remains authoritative.
        if let Some(transaction) = client_transaction.as_ref() {
            transaction.data().completion.record_forced_termination();
        }

        let command_result = if let Some(transaction) = client_transaction.as_ref() {
            transaction
                .data()
                .cmd_tx
                .try_send(InternalTransactionCommand::Terminate)
        } else {
            server_transaction
                .as_ref()
                .expect("one transaction kind was selected")
                .data()
                .cmd_tx
                .try_send(InternalTransactionCommand::Terminate)
        };

        if command_result.is_ok() {
            let initial_send_in_flight = client_transaction
                .as_ref()
                .is_some_and(|transaction| transaction.data().initial_send_in_flight());
            let handle = if let Some(transaction) = client_transaction.as_ref() {
                transaction.data().event_loop_handle.lock().await.take()
            } else {
                server_transaction
                    .as_ref()
                    .expect("one transaction kind was selected")
                    .data()
                    .event_loop_handle
                    .lock()
                    .await
                    .take()
            };
            if let Some(mut handle) = handle {
                if initial_send_in_flight {
                    // The runner set the conservative wire boundary before
                    // awaiting the transport. It cannot consume Terminate until
                    // that await returns, so waiting here only delays fail-closed
                    // cleanup. Abort immediately; the takeover path below keeps
                    // the INVITE route for CANCEL/late-response handling.
                    handle.abort();
                    let _ = handle.await;
                } else if tokio::time::timeout(Duration::from_secs(1), &mut handle)
                    .await
                    .is_err()
                {
                    handle.abort();
                    let _ = handle.await;
                }
            }
            // A successful enqueue or even a terminal RFC state is not a
            // cleanup/publication guarantee. Continue through exact map
            // cleanup and publish only if the joined runner did not deliver.
        }

        // A CompactRetire command may have published the authoritative
        // generation immediately after the first lookup. Preserve its natural
        // deadline for retransmission absorption and exact completion waits.
        if self.compact_non_invite_tombstones.contains_key(tx_id) {
            return Ok(());
        }

        // Full or a successful-but-unprocessed enqueue proves the runner may
        // be inside message/timer processing; Closed can still race its final
        // publication. Quiesce
        // and join that producer before the synthetic fallback can claim or
        // emit any terminal event, so TransactionTerminated remains final.
        if let Some(transaction) = client_transaction.as_ref() {
            if let Some(handle) = transaction.data().event_loop_handle.lock().await.take() {
                handle.abort();
                let _ = handle.await;
            }
        } else if let Some(transaction) = server_transaction.as_ref() {
            if let Some(handle) = transaction.data().event_loop_handle.lock().await.take() {
                handle.abort();
                let _ = handle.await;
            }
        }

        // A compact generation may have been installed while the runner was
        // quiescing. It owns the shared batch claim and remains authoritative
        // until its protocol deadline.
        if self.compact_non_invite_tombstones.contains_key(tx_id) {
            return Ok(());
        }

        #[cfg(test)]
        {
            let gate = super::TERMINATION_TAKEOVER_TEST_GATE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
                .filter(|gate| gate.transaction_id == *tx_id);
            if let Some(gate) = gate {
                gate.runner_joined.notify_one();
                gate.release.notified().await;
            }
        }

        // Full/Closed is the only synthetic fallback. Claim the entire event
        // sequence before changing state; if the runner already claimed it,
        // that runner remains authoritative and this path publishes nothing.
        let (publication, terminal_owner, state, previous_lifecycle_is_active) =
            if let Some(transaction) = client_transaction.as_ref() {
                let data = transaction.data();
                (
                    std::sync::Arc::clone(&data.terminal_event_publication),
                    data.transaction_admission_owner(),
                    std::sync::Arc::clone(&data.state),
                    data.get_lifecycle() == TransactionLifecycle::Active,
                )
            } else {
                let data = server_transaction
                    .as_ref()
                    .expect("one transaction kind was selected")
                    .data();
                (
                    std::sync::Arc::clone(&data.terminal_event_publication),
                    data.transaction_admission_owner(),
                    std::sync::Arc::clone(&data.state),
                    data.get_lifecycle() == TransactionLifecycle::Active,
                )
            };
        let previous_state = state.set(TransactionState::Terminated);
        if previous_state != TransactionState::Terminated && previous_lifecycle_is_active {
            publication.record_prefix(previous_state);
        }
        if let Some(transaction) = client_transaction.as_ref() {
            transaction
                .data()
                .completion
                .record_state(TransactionState::Terminated);
            transaction
                .data()
                .set_lifecycle(TransactionLifecycle::Destroyed);
        } else if let Some(transaction) = server_transaction.as_ref() {
            transaction
                .data()
                .set_lifecycle(TransactionLifecycle::Destroyed);
            // The state is terminal before this barrier. Any response writer
            // already inside the serializer finishes first; a waiter that
            // acquires afterward observes Terminated and is rejected.
            drop(transaction.data().last_response.lock().await);
        }

        let mut publication_claim = if publication.is_delivered() {
            None
        } else {
            publication.try_claim()
        };

        let mut prefix_delivered = true;
        if let (Some(claim), Some(prefix_previous_state)) =
            (publication_claim.as_ref(), publication.pending_prefix())
        {
            let delivery = self.events_tx.send_terminal_prefix(
                crate::transaction::TransactionEvent::StateChanged {
                    transaction_id: tx_id.clone(),
                    previous_state: prefix_previous_state,
                    new_state: TransactionState::Terminated,
                },
                claim,
            );
            if !matches!(
                tokio::time::timeout(Duration::from_secs(1), delivery).await,
                Ok(Ok(()))
            ) {
                self.events_tx.fail_closed_terminal_batch();
                publication_claim
                    .take()
                    .expect("terminal publication claim is live")
                    .mark_failed_closed();
                prefix_delivered = false;
            }
        }

        if client_transaction.is_some() {
            self.retire_and_remove_client_transaction(tx_id).await;
        } else if let Some(transaction) = server_transaction.as_ref() {
            self.server_transactions.remove_if(tx_id, |_, current| {
                std::sync::Arc::ptr_eq(current, transaction)
            });
            self.retire_server_invite_dialog_index_for(tx_id);
        }
        self.terminated_transactions.remove(tx_id);
        if tx_id.is_server() {
            self.transaction_destinations
                .remove_if(tx_id, |_, route| route.is_active());
        }
        self.pending_inbound_bytes.remove(tx_id);
        self.pending_inbound_inserted_at.remove(tx_id);
        self.pending_inbound_transport.remove(tx_id);
        self.pending_inbound_timing.remove(tx_id);
        self.timer_manager.unregister_transaction(tx_id).await;

        if prefix_delivered {
            let Some(publication_claim) = publication_claim else {
                return Ok(());
            };
            let mut delivery = std::pin::pin!(self.events_tx.send_terminal(
                crate::transaction::TransactionEvent::TransactionTerminated {
                    transaction_id: tx_id.clone(),
                },
                None,
                terminal_owner,
            ));
            let delivered = tokio::select! {
                result = &mut delivery => {
                    if result.is_err() {
                        self.events_tx.fail_closed_terminal_batch();
                        false
                    } else {
                        true
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    // `delivery` still owns its exact sidecar/admission owner
                    // while this closes admission. It is dropped only after
                    // the select branch completes, eliminating the ABA gap.
                    self.events_tx.fail_closed_terminal_batch();
                    false
                }
            };
            drop(delivery);
            if delivered {
                publication_claim.mark_delivered();
            } else {
                publication_claim.mark_failed_closed();
            }
        }
        Ok(())
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
    pub async fn cleanup_indexed_terminated_transactions(&self) -> Result<usize> {
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => Ok(0),
            result = self.cleanup_indexed_terminated_transactions_within_operation() => result,
        }
    }

    pub(super) async fn cleanup_indexed_terminated_transactions_within_operation(
        &self,
    ) -> Result<usize> {
        let started = diagnostics::transaction_timing_enabled().then(Instant::now);
        let terminated_keys: Vec<TransactionKey> = self
            .terminated_transactions
            .iter()
            .take(super::TERMINATED_CLEANUP_BATCH_MAX)
            .map(|entry| entry.key().clone())
            .collect();
        let scanned_keys = terminated_keys.len();

        if terminated_keys.is_empty() {
            if let Some(started) = started {
                diagnostics::record_termination_cleanup_indexed_scan(0, started.elapsed());
            }
            return Ok(0);
        }

        debug!(
            "Found {} indexed terminated transactions",
            terminated_keys.len()
        );

        let cleaned = self
            .cleanup_terminated_transaction_keys(terminated_keys, true)
            .await;
        if let Some(started) = started {
            diagnostics::record_termination_cleanup_indexed_scan(scanned_keys, started.elapsed());
        }
        cleaned
    }

    /// Perform an explicit diagnostic repair sweep over all transaction maps.
    ///
    /// Normal runtime cleanup is event-driven and bounded by the terminated
    /// index. This compatibility API deliberately remains available to repair
    /// state after fault injection or to audit invariants, but the manager does
    /// not invoke it from a periodic task because it is O(all transactions).
    pub async fn cleanup_terminated_transactions(&self) -> Result<usize> {
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => Ok(0),
            result = self.cleanup_terminated_transactions_within_operation() => result,
        }
    }

    async fn cleanup_terminated_transactions_within_operation(&self) -> Result<usize> {
        let started = diagnostics::transaction_timing_enabled().then(Instant::now);
        let mut cleaned_count = 0;

        // Cleanup client transactions
        let terminated_client_keys: Vec<TransactionKey> = self
            .client_transactions
            .iter()
            .filter(|r| r.value().state() == TransactionState::Terminated)
            .map(|r| r.key().clone())
            .collect();
        let terminated_client_count = terminated_client_keys.len();
        debug!(
            "Found {} terminated client transactions",
            terminated_client_keys.len()
        );
        cleaned_count += self
            .cleanup_terminated_transaction_keys(terminated_client_keys, false)
            .await?;

        // Cleanup server transactions
        let terminated_server_keys: Vec<TransactionKey> = self
            .server_transactions
            .iter()
            .filter(|r| r.value().state() == TransactionState::Terminated)
            .map(|r| r.key().clone())
            .collect();
        let terminated_server_count = terminated_server_keys.len();
        debug!(
            "Found {} terminated server transactions",
            terminated_server_keys.len()
        );
        cleaned_count += self
            .cleanup_terminated_transaction_keys(terminated_server_keys, false)
            .await?;

        // Cleanup orphaned entries in the transaction_destinations map
        {
            let orphaned_keys: Vec<TransactionKey> = self
                .transaction_destinations
                .iter()
                .filter(|entry| {
                    let k = entry.key();
                    entry.value().is_active()
                        && !self.client_transactions.contains_key(k)
                        && !self.server_transactions.contains_key(k)
                })
                .map(|entry| entry.key().as_ref().clone())
                .collect();
            debug!("Found {} orphaned destination entries", orphaned_keys.len());
            for key in orphaned_keys {
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Removing orphaned destination entry");
                self.transaction_destinations
                    .remove_if(&key, |_, state| state.is_active());
            }
        }

        // Also manually check for client transactions that look terminated but don't have the state set
        {
            let potentially_terminated: Vec<TransactionKey> = self
                .client_transactions
                .iter()
                .filter_map(|r| {
                    let tx = r.value();
                    if tx.as_client_transaction().is_some()
                        && tx.state() == TransactionState::Terminated
                    {
                        Some(r.key().clone())
                    } else {
                        None
                    }
                })
                .collect();

            cleaned_count += self
                .cleanup_terminated_transaction_keys(potentially_terminated, false)
                .await?;
        }

        debug!("Cleaned up {} terminated transactions", cleaned_count);
        if let Some(started) = started {
            diagnostics::record_termination_cleanup_full_scan(
                terminated_client_count,
                terminated_server_count,
                started.elapsed(),
            );
        }
        Ok(cleaned_count)
    }

    async fn cleanup_terminated_transaction_keys(
        &self,
        transaction_keys: Vec<TransactionKey>,
        requeue_active: bool,
    ) -> Result<usize> {
        let mut cleaned_count = 0;
        let mut terminated_transaction_ids = Vec::new();

        for key in transaction_keys {
            self.terminated_transactions.remove(&key);
            let mut removed = false;

            let remove_client = self
                .client_transactions
                .get(&key)
                .map(|entry| entry.value().state() == TransactionState::Terminated)
                .unwrap_or(false);
            if remove_client {
                self.request_transaction_runner_stop(&key);
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Removing terminated client transaction");
                if self.retire_and_remove_client_transaction(&key).await {
                    removed = true;
                }
            }

            let remove_server = self
                .server_transactions
                .get(&key)
                .map(|entry| entry.value().state() == TransactionState::Terminated)
                .unwrap_or(false);
            if remove_server {
                self.request_transaction_runner_stop(&key);
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&key), "Removing terminated server transaction");
                self.server_transactions.remove(&key);
                self.retire_server_invite_dialog_index_for(&key);
                removed = true;
            }

            if removed {
                if key.is_server() {
                    self.transaction_destinations
                        .remove_if(&key, |_, state| state.is_active());
                }
                self.pending_inbound_bytes.remove(&key);
                self.pending_inbound_inserted_at.remove(&key);
                self.pending_inbound_transport.remove(&key);
                self.pending_inbound_timing.remove(&key);
                terminated_transaction_ids.push(key);
                cleaned_count += 1;
            } else if requeue_active
                && (self.client_transactions.contains_key(&key)
                    || self.server_transactions.contains_key(&key))
            {
                self.terminated_transactions.insert(key, ());
            }
        }

        if terminated_transaction_ids.is_empty() {
            return Ok(0);
        }

        let terminated_set: HashSet<TransactionKey> =
            terminated_transaction_ids.iter().cloned().collect();

        // **CRITICAL FIX**: Clean up subscriber mappings for all terminated transactions
        let mut subscriber_ids_to_clean = Vec::new();
        for tx_id in &terminated_transaction_ids {
            if let Some((_, subscriber_ids)) = self.transaction_to_subscribers.remove(tx_id) {
                debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), subscriber_count = subscriber_ids.len(), "Removed terminated transaction from subscriber mappings");
                subscriber_ids_to_clean
                    .extend(subscriber_ids.into_iter().map(|subscriber| subscriber.id));
            }
        }

        if !subscriber_ids_to_clean.is_empty() {
            for subscriber_id in subscriber_ids_to_clean {
                let mut empty = false;
                let mut changed = false;
                let mut new_count = 0;
                let mut old_count = 0;
                if let Some(mut entry) = self.subscriber_to_transactions.get_mut(&subscriber_id) {
                    let tx_list = entry.value_mut();
                    old_count = tx_list.len();
                    tx_list.retain(|tx_id| !terminated_set.contains(tx_id));
                    new_count = tx_list.len();
                    empty = tx_list.is_empty();
                    changed = old_count != new_count;
                }
                if empty {
                    self.subscriber_to_transactions.remove(&subscriber_id);
                    debug!(subscriber_id, "Removed empty subscriber mapping");
                } else if changed {
                    debug!(
                        subscriber_id,
                        old_count, new_count, "Cleaned up subscriber transaction list"
                    );
                }
            }
        }

        // Unregister terminated transactions from timer manager
        for tx_id in &terminated_transaction_ids {
            self.timer_manager.unregister_transaction(tx_id).await;
            debug!(transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Unregistered terminated transaction from timer manager");
        }

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
    pub async fn find_related_transactions(
        &self,
        tx_id: &TransactionKey,
    ) -> Result<Vec<TransactionKey>> {
        let mut related = Vec::new();

        // Get the original request from the transaction
        let request = match self.original_request(tx_id).await? {
            Some(req) => req,
            None => return Ok(Vec::new()), // No request, no related transactions
        };

        // For INVITE transactions, look for related CANCEL transactions
        if request.method() == Method::Invite {
            let cancel_matches: Vec<TransactionKey> = self
                .client_transactions
                .iter()
                .filter(|r| r.key().method() == &Method::Cancel && !r.key().is_server)
                .map(|r| r.key().clone())
                .collect();

            for cancel_key in cancel_matches {
                if let Ok(Some(cancel_req)) = self.original_request(&cancel_key).await {
                    // Check if the CANCEL matches this INVITE
                    if crate::transaction::method::cancel::is_cancel_for_invite(
                        &cancel_req,
                        &request,
                    ) {
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
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => {
                Err(Error::Other("transaction manager stopped request retry".into()))
            }
            result = self.retry_request_within_operation(tx_id) => result,
        }
    }

    async fn retry_request_within_operation(&self, tx_id: &TransactionKey) -> Result<()> {
        if tx_id.is_server() {
            return Err(Error::Other(
                "Cannot retry a server transaction".to_string(),
            ));
        }

        // Extract Arc out of shard before awaiting on original_request.
        let tx = self
            .client_transactions
            .get(tx_id)
            .map(|r| r.value().clone());
        let tx = tx.ok_or_else(|| {
            Error::transaction_not_found(tx_id.clone(), "retry_request - transaction not found")
        })?;

        if let Some(client_tx) = tx.as_client_transaction() {
            let request = client_tx.original_request().await.ok_or_else(|| {
                Error::Other("No original request available for retry".to_string())
            })?;
            let destination = client_tx.remote_addr();
            let transport = self.transport.clone();
            transport
                .send_message(Message::Request(request), destination)
                .await
                .map_err(|e| Error::transport_error(e, "Failed to retry request"))
        } else {
            Err(Error::Other(
                "Failed to downcast to client transaction".to_string(),
            ))
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
        let _operation = self
            .admission_lifecycle
            .try_enter_existing()
            .ok_or_else(|| Error::Other("transaction manager is stopping".into()))?;
        tokio::select! {
            biased;
            _ = self.operation_cancellation.cancelled() => {
                Err(Error::Other("transaction manager stopped request processing".into()))
            }
            result = self.process_request_within_operation(tx_id, request) => result,
        }
    }

    async fn process_request_within_operation(
        &self,
        tx_id: &TransactionKey,
        request: Request,
    ) -> Result<()> {
        if !tx_id.is_server() {
            return Err(Error::Other(
                "Cannot process request for client transaction".to_string(),
            ));
        }

        // Extract Arc out of the shard before awaiting `process_request()`.
        let tx = self
            .server_transactions
            .get(tx_id)
            .map(|r| r.value().clone());

        if let Some(tx) = tx {
            return tx.process_request(request).await;
        }

        if let Some((wire, route)) =
            self.compact_non_invite_tombstones
                .get(tx_id)
                .and_then(|entry| {
                    entry
                        .value()
                        .server_replay()
                        .map(|(wire, route)| (wire.clone(), route.clone()))
                })
        {
            return self
                .transport
                .send_message_raw_via(wire, route)
                .await
                .map_err(|error| {
                    Error::transport_error(
                        error,
                        "Failed to replay compact non-INVITE server response",
                    )
                });
        }

        Err(Error::transaction_not_found(
            tx_id.clone(),
            "process_request - transaction not found",
        ))
    }
}
