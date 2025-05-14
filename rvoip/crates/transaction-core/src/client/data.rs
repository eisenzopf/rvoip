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

/// Common data for client transactions
#[derive(Clone)]
pub struct ClientTransactionData {
    /// Transaction ID
    pub id: TransactionKey,
    /// Current transaction state
    pub state: Arc<AtomicTransactionState>,
    /// Original request
    pub request: Arc<Mutex<Request>>,
    /// Last received response
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

impl Drop for ClientTransactionData {
    fn drop(&mut self) {
        // Try to terminate the event loop when the transaction is dropped
        debug!(id=%self.id, "ClientTransactionData dropped, attempting to terminate event loop");
        
        if let Ok(mut handle_guard) = self.event_loop_handle.try_lock() {
            if let Some(handle) = handle_guard.take() {
                handle.abort();
                debug!(id=%self.id, "Aborted client transaction event loop");
            }
        }
    }
}

/// Common methods for client transactions
pub trait CommonClientTransaction {
    /// Get the shared data for this transaction
    fn data(&self) -> &Arc<ClientTransactionData>;
    
    /// Common process_response implementation
    fn process_response_common(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            trace!(id=%data.id, method=%response.status(), "Received response");
            
            data.cmd_tx.send(InternalTransactionCommand::ProcessMessage(Message::Response(response))).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))?;
            
            Ok(())
        })
    }
}

impl fmt::Debug for ClientTransactionData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientTransactionData")
            .field("id", &self.id)
            .field("state", &self.state.get())
            .field("remote_addr", &self.remote_addr)
            .field("has_event_loop", &self.event_loop_handle.try_lock().map(|h| h.is_some()).unwrap_or(false))
            .finish()
    }
} 