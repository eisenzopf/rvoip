/// # Client Transaction Data Structures
///
/// This module provides data structures and traits for implementing the client transaction
/// state machines defined in RFC 3261 Section 17.1.
///
/// Client transactions in SIP are responsible for:
/// - Reliably sending requests from the Transaction User (TU)
/// - Managing retransmissions over unreliable transports
/// - Receiving and routing responses back to the TU
/// - Maintaining transaction state according to RFC 3261
///
/// The key components in this module are:
/// - `ClientTransactionData`: Core data structure shared by all client transaction types
/// - `CommonClientTransaction`: Trait providing shared behavior across transaction types
/// - Command channels for communication with the transaction's event loop
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::task::JoinHandle;
use tracing::{debug, trace};

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::{Transport, TransportRoute};

use crate::transaction::error::{Error, Result};
use crate::transaction::runner::{
    AsRefKey, AsRefState, HasCommandSender, HasLifecycle, HasTransactionEvents, HasTransport,
};
use crate::transaction::state::TransactionLifecycle;
use crate::transaction::timer::TimerSettings;
use crate::transaction::{
    AtomicTransactionState, InternalTransactionCommand, TransactionEvent, TransactionKey,
    TransactionState,
};

/// Command sender for transaction event loops.
///
/// Used to send commands to the transaction's internal event loop, allowing
/// asynchronous control of the transaction's behavior.
pub type CommandSender = mpsc::Sender<InternalTransactionCommand>;

/// Command receiver for transaction event loops.
///
/// Used by the transaction's event loop to receive commands from other
/// components, such as the TransactionManager or the transaction itself.
pub type CommandReceiver = mpsc::Receiver<InternalTransactionCommand>;

/// Common data structure for both INVITE and non-INVITE client transactions.
///
/// This structure contains all the state required for implementing the client transaction
/// state machines defined in RFC 3261 Section 17.1. It includes:
///
/// - Identity information (transaction key)
/// - State tracking (current transaction state)
/// - Message storage (original request, last response)
/// - Communication channels (transport, event channels, command channels)
/// - Timer configuration
///
/// Both `ClientInviteTransaction` and `ClientNonInviteTransaction` use this structure
/// as their core data store, while implementing different behavior around it.
#[derive(Clone)]
pub struct ClientTransactionData {
    /// Transaction ID based on RFC 3261 transaction matching rules
    pub id: TransactionKey,

    /// Current transaction state (Initial, Calling/Trying, Proceeding, Completed, Terminated)
    pub state: Arc<AtomicTransactionState>,

    /// Transaction lifecycle state for robust shutdown coordination
    pub lifecycle: Arc<std::sync::atomic::AtomicU8>, // Using AtomicU8 for TransactionLifecycle

    /// Original request that initiated this transaction.
    ///
    /// `Arc<Request>` not `Arc<Mutex<Request>>` — the original request
    /// is immutable after construction (RFC 3261 retransmissions send
    /// the same request bytes). Every previous `data.request.lock().await`
    /// was a pointless mutex acquire on read-only data, hit per timer
    /// fire, per retransmit, per state action.
    pub request: Arc<Request>,

    /// Last response received for this transaction
    pub last_response: Arc<Mutex<Option<Response>>>,

    /// Remote address to which requests are sent
    pub remote_addr: SocketAddr,

    /// Authority- and transport-bearing route for every initial send and
    /// retransmission of this client transaction.
    pub request_route: Arc<Mutex<TransportRoute>>,

    /// Transport layer for sending SIP messages
    pub transport: Arc<dyn Transport>,

    /// Channel for sending events to the Transaction User (TU)
    pub events_tx: mpsc::Sender<TransactionEvent>,

    /// Channel for sending commands to the transaction's event loop
    pub cmd_tx: CommandSender,

    /// Handle to the transaction's event loop task
    pub event_loop_handle: Arc<Mutex<Option<JoinHandle<()>>>>,

    /// Configuration for transaction timers (T1, T2, etc.)
    pub timer_config: TimerSettings,

    /// Completion handshake for the first transport write. `initiate()` must
    /// not report success merely because the state-transition command was
    /// queued: RFC 3263 candidate failover depends on the actual initial send
    /// result. 0=pending, 1=sent, 2=failed.
    pub(crate) initial_send_state: Arc<AtomicU8>,
    pub(crate) initial_send_notify: Arc<Notify>,
}

impl Drop for ClientTransactionData {
    fn drop(&mut self) {
        // Try to terminate the event loop when the transaction is dropped
        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&self.id), "ClientTransactionData dropped, attempting to terminate event loop");

        if let Ok(mut handle_guard) = self.event_loop_handle.try_lock() {
            if let Some(handle) = handle_guard.take() {
                handle.abort();
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&self.id), "Aborted client transaction event loop");
            }
        }
    }
}

impl ClientTransactionData {
    pub(crate) fn initial_send_succeeded(&self) -> bool {
        self.initial_send_state.load(Ordering::Acquire) == 1
    }

    pub(crate) fn complete_initial_send(&self, succeeded: bool) {
        let state = if succeeded { 1 } else { 2 };
        if self
            .initial_send_state
            .compare_exchange(0, state, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            // One initiator waits for this one-shot result. `notify_one`
            // retains a permit when completion wins the race with the first
            // poll of `notified()`; `notify_waiters` would lose that wake.
            self.initial_send_notify.notify_one();
        }
    }

    pub(crate) async fn await_initial_send(&self) -> Result<()> {
        loop {
            // Register before reading the state so completion between the
            // read and await cannot be lost.
            let notified = self.initial_send_notify.notified();
            match self.initial_send_state.load(Ordering::Acquire) {
                1 => return Ok(()),
                2 => {
                    return Err(Error::transport_error(
                        rvoip_sip_transport::Error::ProtocolError(
                            "initial request transport send failed".into(),
                        ),
                        "Failed to send initial request",
                    ));
                }
                _ if self.state.get() == TransactionState::Terminated => {
                    return Err(Error::transport_error(
                        rvoip_sip_transport::Error::TransportClosed,
                        "Initial request transaction terminated before transport send",
                    ));
                }
                _ => {
                    // A transaction runner can terminate before entering its
                    // first state (for example when its TU event channel is
                    // already closed), in which case no transport handler can
                    // complete the one-shot. Periodically re-check state so
                    // `initiate()` remains bounded by lifecycle progress.
                    tokio::select! {
                        _ = notified => {}
                        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
                    }
                }
            }
        }
    }

    /// Send on the retained route and atomically retain the concrete selected
    /// flow before the transaction reports the send as successful.
    pub async fn send_on_request_route(&self, message: Message) -> rvoip_sip_transport::Result<()> {
        let route = self.request_route.lock().await.clone();
        let prepared = self
            .transport
            .prepare_message_route(&message, route)
            .await?;
        // Publish the exact flow before the first SIP byte can trigger a
        // response. The event dispatcher can authenticate against this route
        // even if the peer responds and closes immediately.
        *self.request_route.lock().await = prepared.clone();
        let bound = self
            .transport
            .send_message_on_route(message, prepared)
            .await?;
        *self.request_route.lock().await = bound;
        Ok(())
    }
}

/// Common behavior trait for all client transactions.
///
/// This trait provides shared functionality that all client transactions need,
/// regardless of whether they are INVITE or non-INVITE transactions. It serves
/// as a base for the more specific transaction type implementations.
pub trait CommonClientTransaction {
    /// Returns the shared transaction data.
    ///
    /// # Returns
    ///
    /// A reference to the `ClientTransactionData` structure containing the transaction's state.
    fn data(&self) -> &Arc<ClientTransactionData>;

    /// Common implementation for processing responses.
    ///
    /// This method provides a default implementation for the `process_response` method
    /// in the `ClientTransaction` trait, handling the common logic of storing the response
    /// and sending a command to the transaction's event loop.
    ///
    /// # Arguments
    ///
    /// * `response` - The SIP response to process
    ///
    /// # Returns
    ///
    /// A Future that resolves to Ok(()) if the response was processed successfully,
    /// or an Error if there was a problem.
    fn process_response_common(
        &self,
        response: Response,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data().clone();

        Box::pin(async move {
            trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&data.id), method=%response.status(), "Received response");

            data.cmd_tx
                .send(InternalTransactionCommand::ProcessMessage(
                    Message::Response(response),
                ))
                .await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))?;

            Ok(())
        })
    }
}

impl fmt::Debug for ClientTransactionData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientTransactionData")
            .field(
                "id",
                &crate::transaction::safe_diagnostics::SafeTransactionKey::new(&self.id),
            )
            .field("state", &self.state.get())
            .field("remote_addr", &self.remote_addr)
            .field(
                "request_route_available",
                &self.request_route.try_lock().is_ok(),
            )
            .field("request_header_count", &self.request.all_headers().len())
            .field("request_body_len", &self.request.body().len())
            .field(
                "has_last_response",
                &self
                    .last_response
                    .try_lock()
                    .map(|response| response.is_some())
                    .unwrap_or(false),
            )
            .field(
                "has_event_loop",
                &self
                    .event_loop_handle
                    .try_lock()
                    .map(|h| h.is_some())
                    .unwrap_or(false),
            )
            .finish()
    }
}

// Implementation of transaction runner traits for ClientTransactionData

/// Allows access to the transaction state.
/// Required by the transaction runner to manage state transitions.
impl AsRefState for ClientTransactionData {
    fn as_ref_state(&self) -> &Arc<AtomicTransactionState> {
        &self.state
    }
}

/// Allows access to the transaction key.
/// Required by the transaction runner for identification and logging.
impl AsRefKey for ClientTransactionData {
    fn as_ref_key(&self) -> &TransactionKey {
        &self.id
    }
}

/// Provides access to the event channel.
/// Required by the transaction runner to send events to the Transaction User.
impl HasTransactionEvents for ClientTransactionData {
    fn get_tu_event_sender(&self) -> mpsc::Sender<TransactionEvent> {
        self.events_tx.clone()
    }
}

/// Provides access to the transport layer.
/// Required by the transaction runner to send messages.
impl HasTransport for ClientTransactionData {
    fn get_transport_layer(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }
}

/// Provides access to the command channel.
/// Required by the transaction runner to send commands to itself.
impl HasCommandSender for ClientTransactionData {
    fn get_self_command_sender(&self) -> mpsc::Sender<InternalTransactionCommand> {
        self.cmd_tx.clone()
    }
}

/// Implementation of HasLifecycle trait for ClientTransactionData
impl HasLifecycle for ClientTransactionData {
    /// Get the current lifecycle state
    fn get_lifecycle(&self) -> TransactionLifecycle {
        let val = self.lifecycle.load(std::sync::atomic::Ordering::Acquire);
        match val {
            0 => TransactionLifecycle::Active,
            1 => TransactionLifecycle::Terminating,
            2 => TransactionLifecycle::Draining,
            3 => TransactionLifecycle::Destroyed,
            _ => TransactionLifecycle::Active, // Default fallback
        }
    }

    /// Set the lifecycle state
    fn set_lifecycle(&self, new_lifecycle: TransactionLifecycle) {
        let val = match new_lifecycle {
            TransactionLifecycle::Active => 0,
            TransactionLifecycle::Terminating => 1,
            TransactionLifecycle::Draining => 2,
            TransactionLifecycle::Destroyed => 3,
        };
        self.lifecycle
            .store(val, std::sync::atomic::Ordering::Release);
    }

    /// Check if transaction should emit events to TU (not in Terminating/Draining states)
    fn should_emit_events(&self) -> bool {
        matches!(self.get_lifecycle(), TransactionLifecycle::Active)
    }
}
