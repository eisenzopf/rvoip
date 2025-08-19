/// # Server Transaction Data Structures
///
/// This module provides data structures and traits for implementing the server transaction 
/// state machines defined in RFC 3261 Section 17.2.
///
/// Server transactions in SIP are responsible for:
/// - Processing incoming requests from clients
/// - Sending responses reliably
/// - Managing state transitions based on request/response types
/// - Handling retransmissions according to RFC 3261 rules
///
/// The key components in this module are:
/// - `ServerTransactionData`: Core data structure shared by all server transaction types
/// - `CommonServerTransaction`: Trait providing shared behavior across transaction types
/// - Command channels for communication with the transaction's event loop

use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, trace};

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;

use crate::transaction::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionState, TransactionKey, TransactionEvent,
    InternalTransactionCommand, AtomicTransactionState
};
use crate::transaction::timer::TimerSettings;
use crate::transaction::runner::{AsRefState, AsRefKey, HasTransactionEvents, HasTransport, HasCommandSender};

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

/// Common data structure for both INVITE and non-INVITE server transactions.
///
/// This structure contains all the state required for implementing the server transaction
/// state machines defined in RFC 3261 Section 17.2. It includes:
///
/// - Identity information (transaction key)
/// - State tracking (current transaction state)
/// - Message storage (original request, last response)
/// - Communication channels (transport, event channels, command channels)
/// - Timer configuration
///
/// Both `ServerInviteTransaction` and `ServerNonInviteTransaction` use this structure
/// as their core data store, while implementing different behavior around it.
#[derive(Debug)]
pub struct ServerTransactionData {
    /// Transaction ID based on RFC 3261 transaction matching rules
    pub id: TransactionKey,
    
    /// Current transaction state (Trying/Proceeding, Completed, Confirmed, Terminated)
    pub state: Arc<AtomicTransactionState>,
    
    /// Original request that initiated this transaction
    pub request: Arc<Mutex<Request>>,
    
    /// Last response sent by this transaction
    pub last_response: Arc<Mutex<Option<Response>>>,
    
    /// Remote address to which responses are sent
    pub remote_addr: SocketAddr,
    
    /// Transport layer for sending SIP messages
    pub transport: Arc<dyn Transport>,
    
    /// Channel for sending events to the Transaction User (TU)
    pub events_tx: mpsc::Sender<TransactionEvent>,
    
    /// Channel for sending commands to the transaction's event loop
    pub cmd_tx: CommandSender,
    
    /// Channel for receiving commands in the transaction's event loop
    pub cmd_rx: Arc<Mutex<CommandReceiver>>,
    
    /// Handle to the transaction's event loop task
    pub event_loop_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    
    /// Configuration for transaction timers (T1, T2, etc.)
    pub timer_config: TimerSettings,
}

impl Drop for ServerTransactionData {
    fn drop(&mut self) {
        // Try to terminate the event loop when the transaction is dropped
        debug!(id=%self.id, "ServerTransactionData dropped, attempting to terminate event loop");
        
        if let Ok(mut handle_guard) = self.event_loop_handle.try_lock() {
            if let Some(handle) = handle_guard.take() {
                handle.abort();
                debug!(id=%self.id, "Aborted server transaction event loop");
            }
        }
    }
}

/// Common behavior trait for all server transactions.
///
/// This trait provides shared functionality that all server transactions need,
/// regardless of whether they are INVITE or non-INVITE transactions. It serves
/// as a base for the more specific transaction type implementations.
pub trait CommonServerTransaction {
    /// Returns the shared transaction data.
    ///
    /// # Returns
    ///
    /// A reference to the `ServerTransactionData` structure containing the transaction's state.
    fn data(&self) -> &Arc<ServerTransactionData>;
}

// Implementation of transaction runner traits for ServerTransactionData

/// Allows access to the transaction state.
/// Required by the transaction runner to manage state transitions.
impl AsRefState for ServerTransactionData {
    fn as_ref_state(&self) -> &Arc<AtomicTransactionState> {
        &self.state
    }
}

/// Allows access to the transaction key.
/// Required by the transaction runner for identification and logging.
impl AsRefKey for ServerTransactionData {
    fn as_ref_key(&self) -> &TransactionKey {
        &self.id
    }
}

/// Provides access to the event channel.
/// Required by the transaction runner to send events to the Transaction User.
impl HasTransactionEvents for ServerTransactionData {
    fn get_tu_event_sender(&self) -> mpsc::Sender<TransactionEvent> {
        self.events_tx.clone()
    }
}

/// Provides access to the transport layer.
/// Required by the transaction runner to send messages.
impl HasTransport for ServerTransactionData {
    fn get_transport_layer(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }
}

/// Provides access to the command channel.
/// Required by the transaction runner to send commands to itself.
impl HasCommandSender for ServerTransactionData {
    fn get_self_command_sender(&self) -> mpsc::Sender<InternalTransactionCommand> {
        self.cmd_tx.clone()
    }
} 