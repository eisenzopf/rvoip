use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use rvoip_sip_core::{Message, Method, Request, Response, parse_message};
use rvoip_sip_transport::{Transport, TransportEvent};

use crate::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionState, TransactionType,
    client::{ClientTransaction, ClientInviteTransaction, ClientNonInviteTransaction},
    server::{ServerTransaction, ServerInviteTransaction, ServerNonInviteTransaction},
};
use crate::utils;

/// Transaction events sent by the transaction manager
#[derive(Debug, Clone)]
pub enum TransactionEvent {
    /// Transaction was created
    TransactionCreated {
        /// Transaction ID
        transaction_id: String,
    },
    
    /// Transaction was completed
    TransactionCompleted {
        /// Transaction ID
        transaction_id: String,
        /// Final response
        response: Option<Response>,
    },
    
    /// Transaction was terminated
    TransactionTerminated {
        /// Transaction ID
        transaction_id: String,
    },
    
    /// Message was received that did not match any transaction
    UnmatchedMessage {
        /// SIP message
        message: Message,
        /// Source address of the message
        source: SocketAddr,
    },
    
    /// Error occurred in transaction processing
    Error {
        /// Error description
        error: String,
        /// Transaction ID (if available)
        transaction_id: Option<String>,
    },
}

/// Transaction timer data
struct TransactionTimer {
    /// Transaction ID
    transaction_id: String,
    /// When the timer should fire
    expiry: Instant,
}

type BoxedTransaction = Box<dyn Transaction + Send + Sync>;
type BoxedServerTransaction = Box<dyn ServerTransaction + Send + Sync>;
type BoxedClientTransaction = Box<dyn ClientTransaction + Send + Sync>;

/// Manages SIP transactions
pub struct TransactionManager {
    /// Transport to use for messages
    transport: Arc<dyn Transport>,
    /// Active client transactions
    client_transactions: Arc<Mutex<HashMap<String, BoxedClientTransaction>>>,
    /// Active server transactions
    server_transactions: Arc<Mutex<HashMap<String, BoxedServerTransaction>>>,
    /// Event sender
    events_tx: mpsc::Sender<TransactionEvent>,
    /// Additional event senders for subscribers
    event_subscribers: Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
    /// Transport event receiver
    transport_rx: Arc<Mutex<mpsc::Receiver<TransportEvent>>>,
    /// Is the manager running
    running: Arc<Mutex<bool>>,
}

impl TransactionManager {
    /// Create a new transaction manager
    pub async fn new(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
        event_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        let capacity = event_capacity.unwrap_or(100);
        let (events_tx, events_rx) = mpsc::channel(capacity);
        
        let manager = TransactionManager {
            transport: transport.clone(),
            client_transactions: Arc::new(Mutex::new(HashMap::new())),
            server_transactions: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            event_subscribers: Arc::new(Mutex::new(Vec::new())),
            transport_rx: Arc::new(Mutex::new(transport_rx)),
            running: Arc::new(Mutex::new(true)),
        };
        
        // Start the message processing loop
        manager.start_message_loop();
        // Start the timer processing loop
        manager.start_timer_loop();
        
        Ok((manager, events_rx))
    }
    
    /// Start processing incoming transport messages
    fn start_message_loop(&self) {
        let transport = self.transport.clone();
        let client_transactions = self.client_transactions.clone();
        let server_transactions = self.server_transactions.clone();
        let events_tx = self.events_tx.clone();
        let transport_rx = self.transport_rx.clone();
        let event_subscribers = self.event_subscribers.clone();
        let running = self.running.clone();
        
        tokio::spawn(async move {
            debug!("Starting transaction message loop");
            
            while *running.lock().await {
                // Wait for next message from transport
                let event = match transport_rx.lock().await.recv().await {
                    Some(event) => event,
                    None => {
                        debug!("Transport channel closed, stopping message loop");
                        break;
                    }
                };
                
                match event {
                    TransportEvent::MessageReceived { message, source, destination } => {
                        debug!("Received message from {}: {:?}", source, message);
                        
                        // Process the message through existing transactions or create a new one
                        // In SIP, client transactions handle responses and server transactions handle requests
                        match &message {
                            Message::Request(request) => {
                                if Self::process_request(
                                    &message, 
                                    request, 
                                    source, 
                                    destination,
                                    &transport,
                                    &server_transactions,
                                    &events_tx,
                                    &event_subscribers
                                ).await.is_err() {
                                    error!("Failed to process request from {}", source);
                                }
                            },
                            Message::Response(response) => {
                                if Self::process_response(
                                    &message,
                                    response, 
                                    source, 
                                    &client_transactions,
                                    &events_tx,
                                    &event_subscribers
                                ).await.is_err() {
                                    error!("Failed to process response from {}", source);
                                }
                            }
                        }
                    },
                    TransportEvent::Error { error } => {
                        warn!("Transport error: {}", error);
                        
                        // Forward error to application
                        let event = TransactionEvent::Error {
                            error: error.clone(),
                            transaction_id: None,
                        };
                        
                        // Send to primary channel
                        if let Err(e) = events_tx.send(event.clone()).await {
                            error!("Failed to send error event: {}", e);
                        }
                        
                        // Send to all subscribers
                        let mut subscribers = event_subscribers.lock().await.clone();
                        subscribers.retain(|tx| !tx.is_closed());
                        
                        for tx in &subscribers {
                            if let Err(e) = tx.send(event.clone()).await {
                                error!("Failed to send error event to subscriber: {}", e);
                            }
                        }
                        
                        // Update the subscribers list
                        *event_subscribers.lock().await = subscribers;
                    },
                    TransportEvent::Closed => {
                        info!("Transport closed");
                        break;
                    }
                }
            }
            
            debug!("Transaction message loop stopped");
        });
    }
    
    /// Start the timer processing loop
    fn start_timer_loop(&self) {
        let client_transactions = self.client_transactions.clone();
        let server_transactions = self.server_transactions.clone();
        let events_tx = self.events_tx.clone();
        let running = self.running.clone();
        
        tokio::spawn(async move {
            debug!("Starting transaction timer loop");
            
            // Check for timers every 100ms
            let check_interval = Duration::from_millis(100);
            
            while *running.lock().await {
                // Sleep for the check interval
                sleep(check_interval).await;
                
                // Check client transaction timers
                {
                    let mut client_txs = client_transactions.lock().await;
                    let mut expired_txs = Vec::new();
                    
                    // Find transactions with expired timers
                    for (id, tx) in client_txs.iter_mut() {
                        if let Some(_duration) = tx.timeout_duration() {
                            // Fire timer event
                            match tx.on_timeout().await {
                                Ok(Some(message)) => {
                                    // Forward the message to the application
                                    if let Err(e) = events_tx.send(TransactionEvent::UnmatchedMessage {
                                        message,
                                        source: SocketAddr::from(([127, 0, 0, 1], 5060)), // Dummy source
                                    }).await {
                                        error!("Failed to send timeout message event: {}", e);
                                    }
                                },
                                Ok(None) => {
                                    // No message to forward
                                },
                                Err(e) => {
                                    error!("Error in client transaction timeout: {}", e);
                                    
                                    // Forward error to application
                                    if let Err(e) = events_tx.send(TransactionEvent::Error {
                                        error: e.to_string(),
                                        transaction_id: Some(id.clone()),
                                    }).await {
                                        error!("Failed to send error event: {}", e);
                                    }
                                }
                            }
                            
                            // Check if transaction is terminated
                            if tx.is_terminated() {
                                expired_txs.push(id.clone());
                                
                                // Notify of transaction termination
                                if let Err(e) = events_tx.send(TransactionEvent::TransactionTerminated {
                                    transaction_id: id.clone(),
                                }).await {
                                    error!("Failed to send transaction terminated event: {}", e);
                                }
                            }
                        }
                    }
                    
                    // Remove expired transactions
                    for id in expired_txs {
                        client_txs.remove(&id);
                        debug!("Removed expired client transaction: {}", id);
                    }
                }
                
                // Check server transaction timers
                {
                    let mut server_txs = server_transactions.lock().await;
                    let mut expired_txs = Vec::new();
                    
                    // Find transactions with expired timers
                    for (id, tx) in server_txs.iter_mut() {
                        if let Some(_duration) = tx.timeout_duration() {
                            // Fire timer event
                            if let Err(e) = tx.on_timeout().await {
                                error!("Error in server transaction timeout: {}", e);
                                
                                // Forward error to application
                                if let Err(e) = events_tx.send(TransactionEvent::Error {
                                    error: e.to_string(),
                                    transaction_id: Some(id.clone()),
                                }).await {
                                    error!("Failed to send error event: {}", e);
                                }
                            }
                            
                            // Check if transaction is terminated
                            if tx.is_terminated() {
                                expired_txs.push(id.clone());
                                
                                // Notify of transaction termination
                                if let Err(e) = events_tx.send(TransactionEvent::TransactionTerminated {
                                    transaction_id: id.clone(),
                                }).await {
                                    error!("Failed to send transaction terminated event: {}", e);
                                }
                            }
                        }
                    }
                    
                    // Remove expired transactions
                    for id in expired_txs {
                        server_txs.remove(&id);
                        debug!("Removed expired server transaction: {}", id);
                    }
                }
            }
            
            debug!("Transaction timer loop stopped");
        });
    }
    
    /// Process an incoming request
    async fn process_request(
        message: &Message,
        request: &Request,
        source: SocketAddr,
        destination: SocketAddr,
        transport: &Arc<dyn Transport>,
        server_transactions: &Arc<Mutex<HashMap<String, BoxedServerTransaction>>>,
        events_tx: &mpsc::Sender<TransactionEvent>,
        event_subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
    ) -> Result<()> {
        // Check for existing transaction
        let mut transactions = server_transactions.lock().await;
        
        // Special case for ACK - it's matched with the INVITE transaction
        if request.method == Method::Ack {
            for tx in transactions.values_mut() {
                if tx.matches(message) {
                    debug!("[{}] Found matching server transaction for ACK", tx.id());
                    tx.process_message(message.clone()).await?;
                    
                    // ACK was handled, no need to create a new transaction
                    return Ok(());
                }
            }
            
            // If no matching transaction was found, ACK might be for a 2xx response
            // which is handled by the TU directly (outside of transactions)
            debug!("ACK not matched with any transaction, forwarding to upper layer");
            
            // Forward to application
            events_tx.send(TransactionEvent::UnmatchedMessage {
                message: message.clone(),
                source,
            }).await?;
            
            return Ok(());
        }
        
        // Try to match with existing transaction
        for tx in transactions.values_mut() {
            if tx.matches(message) {
                debug!("[{}] Found matching server transaction", tx.id());
                tx.process_message(message.clone()).await?;
                
                // Message was handled by existing transaction
                return Ok(());
            }
        }
        
        // No matching transaction found, create a new one
        debug!("Creating new server transaction for {} request", request.method);
        
        // Different transaction types based on the method
        let tx: BoxedServerTransaction = if request.method == Method::Invite {
            Box::new(ServerInviteTransaction::new(
                request.clone(),
                source,
                transport.clone(),
            )?)
        } else {
            Box::new(ServerNonInviteTransaction::new(
                request.clone(),
                source,
                transport.clone(),
            )?)
        };
        
        let transaction_id = tx.id().to_string();
        debug!("[{}] Created new server transaction", transaction_id);
        
        // Add transaction to collection
        transactions.insert(transaction_id.clone(), tx);
        
        // Notify of transaction creation
        let created_event = TransactionEvent::TransactionCreated {
            transaction_id: transaction_id.clone(),
        };
        
        // Send to primary channel
        events_tx.send(created_event.clone()).await?;
        
        // Send to subscribers
        for tx in &*event_subscribers.lock().await {
            tx.send(created_event.clone()).await.ok(); // Ignore errors
        }
        
        // Forward message to application for processing
        let unmatched_event = TransactionEvent::UnmatchedMessage {
            message: message.clone(),
            source,
        };
        
        // Send to primary channel
        events_tx.send(unmatched_event.clone()).await?;
        
        // Send to subscribers
        for tx in &*event_subscribers.lock().await {
            tx.send(unmatched_event.clone()).await.ok(); // Ignore errors
        }
        
        Ok(())
    }
    
    /// Process an incoming response
    async fn process_response(
        message: &Message,
        response: &Response,
        source: SocketAddr,
        client_transactions: &Arc<Mutex<HashMap<String, BoxedClientTransaction>>>,
        events_tx: &mpsc::Sender<TransactionEvent>,
        event_subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
    ) -> Result<()> {
        // Try to match with existing transaction
        let mut transactions = client_transactions.lock().await;
        
        for tx in transactions.values_mut() {
            if tx.matches(message) {
                debug!("[{}] Found matching client transaction", tx.id());
                tx.process_message(message.clone()).await?;
                
                // For any final response, emit the completion event
                // This ensures the application is always notified of responses
                if response.status.is_success() || response.status.is_error() {
                    debug!("[{}] Got final response ({}), emitting TransactionCompleted event", 
                           tx.id(), response.status);
                    
                    // Create completion event
                    let completed_event = TransactionEvent::TransactionCompleted {
                        transaction_id: tx.id().to_string(),
                        response: Some(response.clone()),
                    };
                    
                    // Send to primary channel
                    events_tx.send(completed_event.clone()).await?;
                    
                    // Send to subscribers
                    for tx in &*event_subscribers.lock().await {
                        tx.send(completed_event.clone()).await.ok(); // Ignore errors
                    }
                }
                
                // Message was handled by existing transaction
                return Ok(());
            }
        }
        
        // No matching transaction, this might be a stray response or a new 2xx for INVITE
        debug!("Response not matched with any transaction, forwarding to upper layer");
        
        // Forward to application
        events_tx.send(TransactionEvent::UnmatchedMessage {
            message: message.clone(),
            source,
        }).await?;
        
        Ok(())
    }
    
    /// Create a new client INVITE transaction
    pub async fn create_client_invite_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<String> {
        // Ensure the request is an INVITE
        if request.method != Method::Invite {
            return Err(Error::Other("Request must be INVITE for client INVITE transaction".to_string()));
        }
        
        // Create the transaction
        let mut transaction = ClientInviteTransaction::new(
            request,
            destination,
            self.transport.clone(),
        )?;
        
        let transaction_id = transaction.id().to_string();
        debug!("[{}] Created new client INVITE transaction", transaction_id);
        
        // Add transaction to collection
        self.client_transactions.lock().await.insert(transaction_id.clone(), Box::new(transaction));
        
        // Notify of transaction creation
        self.events_tx.send(TransactionEvent::TransactionCreated {
            transaction_id: transaction_id.clone(),
        }).await?;
        
        Ok(transaction_id)
    }
    
    /// Create a new client non-INVITE transaction
    pub async fn create_client_non_invite_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<String> {
        // Ensure the request is not an INVITE or ACK
        if request.method == Method::Invite || request.method == Method::Ack {
            return Err(Error::Other(format!(
                "Request cannot be {} for client non-INVITE transaction", 
                request.method
            )));
        }
        
        // Create the transaction
        let mut transaction = ClientNonInviteTransaction::new(
            request,
            destination,
            self.transport.clone(),
        )?;
        
        let transaction_id = transaction.id().to_string();
        debug!("[{}] Created new client non-INVITE transaction", transaction_id);
        
        // Add transaction to collection
        self.client_transactions.lock().await.insert(transaction_id.clone(), Box::new(transaction));
        
        // Notify of transaction creation
        self.events_tx.send(TransactionEvent::TransactionCreated {
            transaction_id: transaction_id.clone(),
        }).await?;
        
        Ok(transaction_id)
    }
    
    /// Send a request through a client transaction
    pub async fn send_request(&self, transaction_id: &str) -> Result<()> {
        let mut transactions = self.client_transactions.lock().await;
        
        if let Some(tx) = transactions.get_mut(transaction_id) {
            debug!("[{}] Sending request", transaction_id);
            tx.send_request().await?;
            Ok(())
        } else {
            Err(Error::TransactionNotFound(transaction_id.to_string()))
        }
    }
    
    /// Send a response through a server transaction
    pub async fn send_response(&self, transaction_id: &str, response: Response) -> Result<()> {
        let mut transactions = self.server_transactions.lock().await;
        
        if let Some(tx) = transactions.get_mut(transaction_id) {
            debug!("[{}] Sending response: {}", transaction_id, response.status);
            tx.send_response(response).await?;
            Ok(())
        } else {
            Err(Error::TransactionNotFound(transaction_id.to_string()))
        }
    }
    
    /// Get the transaction state
    pub async fn transaction_state(&self, transaction_id: &str) -> Result<TransactionState> {
        // Check client transactions
        let client_txs = self.client_transactions.lock().await;
        if let Some(tx) = client_txs.get(transaction_id) {
            return Ok(tx.state());
        }
        
        // Check server transactions
        let server_txs = self.server_transactions.lock().await;
        if let Some(tx) = server_txs.get(transaction_id) {
            return Ok(tx.state());
        }
        
        Err(Error::TransactionNotFound(transaction_id.to_string()))
    }
    
    /// Get the transaction type
    pub async fn transaction_type(&self, transaction_id: &str) -> Result<TransactionType> {
        // Check client transactions
        let client_txs = self.client_transactions.lock().await;
        if let Some(tx) = client_txs.get(transaction_id) {
            return Ok(tx.transaction_type());
        }
        
        // Check server transactions
        let server_txs = self.server_transactions.lock().await;
        if let Some(tx) = server_txs.get(transaction_id) {
            return Ok(tx.transaction_type());
        }
        
        Err(Error::TransactionNotFound(transaction_id.to_string()))
    }
    
    /// Get all active transaction IDs
    pub async fn active_transactions(&self) -> (Vec<String>, Vec<String>) {
        let client_txs = self.client_transactions.lock().await;
        let server_txs = self.server_transactions.lock().await;
        
        (
            client_txs.keys().cloned().collect(),
            server_txs.keys().cloned().collect(),
        )
    }
    
    /// Get the transport
    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }
    
    /// Shutdown the transaction manager
    pub async fn shutdown(&self) {
        // Set running flag to false
        let mut running = self.running.lock().await;
        *running = false;
        
        // Clear all transactions
        self.client_transactions.lock().await.clear();
        self.server_transactions.lock().await.clear();
        
        debug!("Transaction manager shutdown");
    }
    
    /// Subscribe to transaction events
    /// Returns a new receiver that will receive a copy of all transaction events
    pub fn subscribe(&self) -> mpsc::Receiver<TransactionEvent> {
        // Create a new channel for events
        let (tx, rx) = mpsc::channel(100);
        
        // Add the sender to our list of subscribers
        tokio::spawn({
            let event_subscribers = self.event_subscribers.clone();
            let tx = tx.clone();
            async move {
                event_subscribers.lock().await.push(tx);
            }
        });
        
        rx
    }
    
    /// Create a dummy TransactionManager for testing
    pub fn dummy() -> Self {
        use crate::DummyTransport;
        
        let (events_tx, _) = mpsc::channel(1);
        let (_, transport_rx) = mpsc::channel(1);
        
        Self {
            transport: Arc::new(DummyTransport {}),
            client_transactions: Arc::new(Mutex::new(HashMap::new())),
            server_transactions: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            event_subscribers: Arc::new(Mutex::new(Vec::new())),
            transport_rx: Arc::new(Mutex::new(transport_rx)),
            running: Arc::new(Mutex::new(true)),
        }
    }
}

impl fmt::Debug for TransactionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransactionManager")
            .field("transport", &self.transport)
            .field("client_transactions", &self.client_transactions)
            .field("server_transactions", &self.server_transactions)
            .field("events_tx", &self.events_tx)
            .field("event_subscribers", &self.event_subscribers)
            .field("transport_rx", &self.transport_rx)
            .field("running", &self.running)
            .finish()
    }
} 