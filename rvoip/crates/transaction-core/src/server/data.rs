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

use crate::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionState, TransactionKey, TransactionEvent,
    InternalTransactionCommand, AtomicTransactionState
};
use crate::timer::TimerSettings;

/// Command sender type for transaction
pub type CommandSender = mpsc::Sender<InternalTransactionCommand>;
/// Command receiver type for transaction
pub type CommandReceiver = mpsc::Receiver<InternalTransactionCommand>;

/// Common data for server transactions
#[derive(Debug)]
pub struct ServerTransactionData {
    /// Transaction ID
    pub id: TransactionKey,
    /// Current transaction state
    pub state: Arc<AtomicTransactionState>,
    /// Original request
    pub request: Arc<Mutex<Request>>,
    /// Last sent response
    pub last_response: Arc<Mutex<Option<Response>>>,
    /// Remote address to send messages to
    pub remote_addr: SocketAddr,
    /// Transport for sending messages
    pub transport: Arc<dyn Transport>,
    /// Channel for sending events to the transaction user
    pub events_tx: mpsc::Sender<TransactionEvent>,
    /// Channel for sending commands to the transaction's event loop
    pub cmd_tx: CommandSender,
    /// Channel for receiving commands in the transaction's event loop
    pub cmd_rx: Arc<Mutex<CommandReceiver>>,
    /// Handle to the event loop task
    pub event_loop_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Timer configuration
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

/// Common methods for server transactions
pub trait CommonServerTransaction {
    /// Get the shared data for this transaction
    fn data(&self) -> &Arc<ServerTransactionData>;
} 