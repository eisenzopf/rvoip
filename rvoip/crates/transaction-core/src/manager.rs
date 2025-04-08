use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::str::FromStr;
use std::pin::Pin;

use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use rvoip_sip_core::{Message, Method, Request, Response, HeaderName};
use rvoip_sip_transport::{Transport, TransportEvent};

use crate::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionState, TransactionType,
    client::{ClientTransaction, ClientInviteTransaction, ClientNonInviteTransaction},
    server::{ServerTransaction, ServerInviteTransaction, ServerNonInviteTransaction},
};
use crate::transaction::client;
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
    
    /// Response was received for a transaction
    ResponseReceived {
        /// SIP message
        message: Message,
        /// Source address of the message
        source: SocketAddr,
        /// Transaction ID
        transaction_id: String,
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
    /// Transaction destinations - maps transaction IDs to their destinations
    transaction_destinations: Arc<Mutex<HashMap<String, SocketAddr>>>,
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
            transaction_destinations: Arc::new(Mutex::new(HashMap::new())),
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
    
    /// Create a dummy transaction manager for use in situations where a real one is not needed
    /// This should only be used for non-functional instances (like in WeakCall::upgrade)
    pub fn dummy(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
    ) -> Self {
        let (events_tx, _) = mpsc::channel(10);
        
        TransactionManager {
            transport,
            client_transactions: Arc::new(Mutex::new(HashMap::new())),
            server_transactions: Arc::new(Mutex::new(HashMap::new())),
            transaction_destinations: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            event_subscribers: Arc::new(Mutex::new(Vec::new())),
            transport_rx: Arc::new(Mutex::new(transport_rx)),
            running: Arc::new(Mutex::new(false)), // Not running
        }
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
        _destination: SocketAddr,
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
        let mut found_match = false;
        let mut matched_transaction_id = String::new();
        
        // First, try to match with existing transactions
        for (id, tx) in transactions.iter_mut() {
            if tx.matches(message) {
                debug!("[{}] Found matching client transaction", id);
                tx.process_message(message.clone()).await?;
                found_match = true;
                matched_transaction_id = id.clone();
                break;
            }
        }
        
        // If no match found, forward as unmatched
        if !found_match {
            debug!("No matching transaction found for response");
            
            // Create event with additional transaction ID information
            let mut event = TransactionEvent::UnmatchedMessage {
                message: message.clone(),
                source,
            };
            
            // Send to primary channel
            events_tx.send(event.clone()).await?;
            
            // Send to all subscribers
            for tx in &*event_subscribers.lock().await {
                tx.send(event.clone()).await.ok(); // Ignore errors
            }
        } else {
            // Even though the transaction matched, also send it directly
            // This helps with 2xx responses to INVITE that need special handling
            // Include the transaction ID with the unmatched message
            
            // Create a new event including the transaction ID
            let event = TransactionEvent::ResponseReceived {
                message: message.clone(),
                source,
                transaction_id: matched_transaction_id, 
            };
            
            // Send to primary channel
            events_tx.send(event.clone()).await?;
            
            // Send to all subscribers
            for tx in &*event_subscribers.lock().await {
                tx.send(event.clone()).await.ok(); // Ignore errors
            }
        }
        
        Ok(())
    }
    
    /// Create a client transaction
    pub async fn create_client_transaction(&self, 
        request: Request, 
        destination: SocketAddr
    ) -> Result<String> {
        debug!("Creating client transaction for {} request to {}", request.method, destination);
        
        // Different transaction types based on the method
        let tx: BoxedClientTransaction = if request.method == Method::Invite {
            debug!("Creating INVITE client transaction to {}", destination);
            Box::new(ClientInviteTransaction::new(
                request.clone(),
                destination,
                self.transport.clone(),
            )?)
        } else {
            debug!("Creating non-INVITE client transaction to {}", destination);
            Box::new(ClientNonInviteTransaction::new(
                request.clone(),
                destination,
                self.transport.clone(),
            )?)
        };
        
        // Get transaction ID
        let id = tx.id().to_string();
        debug!("Created client transaction with ID: {}", id);
        
        // Store the transaction
        let mut client_txs = self.client_transactions.lock().await;
        client_txs.insert(id.clone(), tx);
        
        // Store the destination address
        let mut destinations = self.transaction_destinations.lock().await;
        destinations.insert(id.clone(), destination);
        debug!("Stored destination {} for transaction {}", destination, id);
        
        // Notify of transaction creation
        self.events_tx.send(TransactionEvent::TransactionCreated {
            transaction_id: id.clone(),
        }).await?;
        
        Ok(id)
    }
    
    /// Lookup a transaction destination
    pub async fn get_transaction_destination(&self, transaction_id: &str) -> Option<SocketAddr> {
        let destinations = self.transaction_destinations.lock().await;
        destinations.get(transaction_id).copied()
    }
    
    /// Send a request through a client transaction
    pub async fn send_request(&self, transaction_id: &str) -> Result<()> {
        use tracing::{info, debug, error};
        
        debug!("Sending request for transaction: {}", transaction_id);
        
        // Find transaction
        let mut transactions = self.client_transactions.lock().await;
        let transaction = transactions.get_mut(transaction_id)
            .ok_or_else(|| Error::TransactionNotFound(format!("Transaction {} not found", transaction_id)))?;
        
        // Cast to ClientTransaction trait (downcast will always succeed for client transactions)
        let client_transaction = transaction.as_mut() as &mut dyn client::ClientTransaction;
        
        // Get the request and destination for logging
        let request = client_transaction.original_request().clone();
        let destination = if let Some(addr) = utils::extract_destination(transaction_id) {
            addr
        } else {
            return Err(Error::Other(format!("Could not extract destination from transaction {}", transaction_id)));
        };
        
        // Log the request details
        info!("Sending {} request to {} for transaction {}", 
            request.method, destination, transaction_id);
        
        // Send the request via the client transaction
        match client_transaction.send_request().await {
            Ok(_) => {
                debug!("Request sent successfully for transaction {}", transaction_id);
                Ok(())
            },
            Err(e) => {
                error!("Failed to send request for transaction {}: {}", transaction_id, e);
                Err(Error::Other(format!("Failed to send request: {}", e)))
            }
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
    
    /// Send an ACK for a 2xx response (outside of transaction)
    pub async fn send_2xx_ack(&self, transaction_id: &str, response: &rvoip_sip_core::Response) -> Result<()> {
        // Create a basic ACK request based on the response
        // This is a simplified version - in a real implementation, we would create a proper ACK
        let mut ack = rvoip_sip_core::Request::new(
            rvoip_sip_core::Method::Ack,
            response.to().and_then(|to_str| 
                rvoip_sip_core::Uri::from_str(to_str).ok()
            ).unwrap_or_else(|| {
                rvoip_sip_core::Uri::sip("localhost")
            })
        );
        
        // Copy required headers from response
        if let Some(call_id) = response.header(&rvoip_sip_core::HeaderName::CallId) {
            ack.headers.push(call_id.clone());
        }
        
        if let Some(from) = response.header(&rvoip_sip_core::HeaderName::From) {
            ack.headers.push(from.clone());
        }
        
        if let Some(to) = response.header(&rvoip_sip_core::HeaderName::To) {
            ack.headers.push(to.clone());
        }
        
        if let Some(via) = response.header(&rvoip_sip_core::HeaderName::Via) {
            ack.headers.push(via.clone());
        }
        
        // Add CSeq
        if let Some(cseq) = response.header(&rvoip_sip_core::HeaderName::CSeq) {
            if let Some(cseq_value) = cseq.value.as_text() {
                if let Some(cseq_num) = cseq_value.split_whitespace().next() {
                    ack.headers.push(rvoip_sip_core::Header::text(
                        rvoip_sip_core::HeaderName::CSeq,
                        format!("{} ACK", cseq_num)
                    ));
                }
            }
        }
        
        // Add Max-Forwards
        ack.headers.push(rvoip_sip_core::Header::integer(
            rvoip_sip_core::HeaderName::MaxForwards,
            70
        ));
        
        // Add Content-Length
        ack.headers.push(rvoip_sip_core::Header::integer(
            rvoip_sip_core::HeaderName::ContentLength,
            0
        ));
        
        // 1. First try to extract destination from Contact header (preferred SIP way)
        let mut destination_from_contact = None;
        if let Some(contact) = response.header(&rvoip_sip_core::HeaderName::Contact) {
            if let Some(contact_text) = contact.value.as_text() {
                info!("Contact header in 200 OK: {}", contact_text);
                if let Some(uri_start) = contact_text.find('<') {
                    let uri_start = uri_start + 1;
                    if let Some(uri_end) = contact_text[uri_start..].find('>') {
                        let uri_str = &contact_text[uri_start..(uri_start + uri_end)];
                        info!("Extracted URI from Contact: {}", uri_str);
                        
                        if let Ok(uri) = rvoip_sip_core::Uri::from_str(uri_str) {
                            // Determine destination address from URI
                            let host = uri.host.clone();
                            let port = uri.port.unwrap_or(5060);
                            info!("URI host: {}, port: {}", host, port);
                            
                            // Try to resolve host as an IP address
                            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                                destination_from_contact = Some(std::net::SocketAddr::new(ip, port));
                                info!("Successfully extracted destination from Contact: {}", destination_from_contact.unwrap());
                            }
                        }
                    }
                }
            }
        }
        
        // 2. Try to extract from Via if Contact failed
        let mut destination_from_via = None;
        if destination_from_contact.is_none() {
            if let Some(via) = response.header(&rvoip_sip_core::HeaderName::Via) {
                if let Some(via_text) = via.value.as_text() {
                    info!("Via header in 200 OK: {}", via_text);
                    // Extract received and rport parameters
                    let mut received = None;
                    let mut rport = None;
                    
                    if let Some(received_pos) = via_text.find("received=") {
                        let received_start = received_pos + 9; // "received=" length
                        let received_end = via_text[received_start..]
                            .find(|c: char| c == ';' || c == ',' || c.is_whitespace())
                            .map(|pos| received_start + pos)
                            .unwrap_or(via_text.len());
                        received = Some(via_text[received_start..received_end].to_string());
                    }
                    
                    if let Some(rport_pos) = via_text.find("rport=") {
                        let rport_start = rport_pos + 6; // "rport=" length
                        let rport_end = via_text[rport_start..]
                            .find(|c: char| c == ';' || c == ',' || c.is_whitespace())
                            .map(|pos| rport_start + pos)
                            .unwrap_or(via_text.len());
                        let rport_str = &via_text[rport_start..rport_end];
                        rport = rport_str.parse::<u16>().ok();
                    }
                    
                    if let (Some(ip_str), Some(port)) = (received, rport) {
                        if let Ok(ip) = ip_str.parse::<std::net::IpAddr>() {
                            destination_from_via = Some(std::net::SocketAddr::new(ip, port));
                            info!("Successfully extracted destination from Via: {}", destination_from_via.unwrap());
                        }
                    }
                }
            }
        }
        
        // 3. Look up from transaction registry
        let destination_from_registry = self.get_transaction_destination(transaction_id).await;
        if let Some(addr) = destination_from_registry {
            info!("Found destination in transaction registry: {}", addr);
        }
        
        // Determine the best destination to use - order of preference:
        // 1. Contact header (most reliable for 2xx responses)
        // 2. Via header with received/rport
        // 3. Transaction registry
        // 4. Fallback hardcoded address
        let destination = destination_from_contact
            .or(destination_from_via)
            .or(destination_from_registry)
            .unwrap_or_else(|| {
                let fallback = std::net::SocketAddr::from(([127, 0, 0, 1], 5071));
                warn!("No reliable destination found for ACK, using fallback: {}", fallback);
                fallback
            });
        
        info!("Sending ACK for 2xx response to {}", destination);
        
        // Send the ACK directly via transport
        self.transport.send_message(
            rvoip_sip_core::Message::Request(ack),
            destination
        ).await.map_err(|e| Error::Other(format!("Failed to send ACK: {}", e)))
    }
}

impl fmt::Debug for TransactionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransactionManager")
            .field("transport", &self.transport)
            .field("client_transactions", &self.client_transactions)
            .field("server_transactions", &self.server_transactions)
            .field("transaction_destinations", &self.transaction_destinations)
            .field("events_tx", &self.events_tx)
            .field("event_subscribers", &self.event_subscribers)
            .field("transport_rx", &self.transport_rx)
            .field("running", &self.running)
            .finish()
    }
} 