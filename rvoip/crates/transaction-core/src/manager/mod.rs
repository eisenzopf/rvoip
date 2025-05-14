mod handlers;
mod types;
mod utils;

pub use types::*;
use handlers::*;
use utils::*;

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::str::FromStr;
use std::pin::Pin;
use std::future::Future;

use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use async_trait::async_trait;

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{Host, TypedHeader};
use rvoip_sip_transport::{Transport, TransportEvent};

use crate::error::{self, Error, Result};
use crate::transaction::{
    Transaction, TransactionAsync, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand,
};
use crate::client::{ClientTransaction, ClientInviteTransaction, ClientNonInviteTransaction, TransactionExt};
use crate::server::{ServerTransaction, ServerInviteTransaction, ServerNonInviteTransaction};
use crate::timer::{Timer, TimerManager, TimerFactory, TimerSettings};
use crate::utils::{transaction_key_from_message, generate_branch, extract_cseq, create_ack_from_invite};

// Type aliases without Sync requirement
type BoxedTransaction = Box<dyn Transaction + Send>;
type BoxedServerTransaction = Box<dyn ServerTransaction + Send>;
type BoxedClientTransaction = Box<dyn ClientTransaction + Send>;

/// Manages SIP transactions
#[derive(Clone)]
pub struct TransactionManager {
    /// Transport to use for messages
    transport: Arc<dyn Transport>,
    /// Active client transactions
    client_transactions: Arc<Mutex<HashMap<TransactionKey, BoxedClientTransaction>>>,
    /// Active server transactions
    server_transactions: Arc<Mutex<HashMap<TransactionKey, BoxedServerTransaction>>>,
    /// Transaction destinations - maps transaction IDs to their destinations
    transaction_destinations: Arc<Mutex<HashMap<TransactionKey, SocketAddr>>>,
    /// Event sender
    events_tx: mpsc::Sender<TransactionEvent>,
    /// Additional event subscribers
    event_subscribers: Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
    /// Transport message channel
    transport_rx: Arc<Mutex<mpsc::Receiver<TransportEvent>>>,
    /// Running flag
    running: Arc<Mutex<bool>>,
    /// Timer configuration
    timer_settings: TimerSettings,
    /// Centralized timer manager
    timer_manager: Arc<TimerManager>,
    /// Timer factory
    timer_factory: TimerFactory,
}

// Define RFC3261 Branch magic cookie
pub const RFC3261_BRANCH_MAGIC_COOKIE: &str = "z9hG4bK";

impl TransactionManager {
    /// Create a new transaction manager 
    pub async fn new(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        let events_capacity = capacity.unwrap_or(100);
        let (events_tx, events_rx) = mpsc::channel(events_capacity);
        
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(Mutex::new(false));
        
        let timer_settings = TimerSettings::default();
        
        // Setup timer manager
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        
        // Create timer factory with the timer manager
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        let manager = Self {
            transport,
            client_transactions,
            server_transactions,
            transaction_destinations,
            events_tx,
            event_subscribers,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
        };
        
        // Start the message processing loop
        manager.start_message_loop();
        
        Ok((manager, events_rx))
    }

    /// Create a new transaction manager with custom timer configuration 
    pub async fn new_with_config(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
        timer_settings: Option<TimerSettings>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        let events_capacity = capacity.unwrap_or(100);
        let (events_tx, events_rx) = mpsc::channel(events_capacity);
        
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(Mutex::new(false));
        
        // Create timer settings
        let timer_settings = timer_settings.unwrap_or_default();
        
        // Create the timer manager with custom config
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        let manager = Self {
            transport,
            client_transactions,
            server_transactions,
            transaction_destinations,
            events_tx,
            event_subscribers,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
        };
        
        // Start the message processing loop
        manager.start_message_loop();
        
        Ok((manager, events_rx))
    }

    /// Create a new transaction manager with classic signature (non-async)
    pub fn new_sync(transport: Arc<dyn Transport>) -> Self {
        Self::with_config(transport, None)
    }
    
    /// Create a new transaction manager with custom timer configuration
    pub fn with_config(transport: Arc<dyn Transport>, timer_settings_opt: Option<TimerSettings>) -> Self {
        let (events_tx, _) = mpsc::channel(100);
        let (_, transport_rx) = mpsc::channel(100);
        
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(Mutex::new(false));
        
        // Create timer settings
        let timer_settings = timer_settings_opt.unwrap_or_else(TimerSettings::default);
        
        // Create the timer manager with custom config
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        Self {
            transport,
            client_transactions,
            server_transactions,
            transaction_destinations,
            events_tx,
            event_subscribers,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
        }
    }

    /// Construct a dummy manager for testing
    pub fn dummy(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
    ) -> Self {
        // Setup basic channels
        let (events_tx, _) = mpsc::channel(10);
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        
        // Transaction registries
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        
        // Setup timer manager
        let timer_settings = TimerSettings::default();
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        // Initialize running state
        let running = Arc::new(Mutex::new(false));
        
        // Track destinations
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        
        Self {
            transport,
            events_tx,
            event_subscribers,
            client_transactions,
            server_transactions,
            timer_factory,
            timer_manager,
            timer_settings,
            running,
            transaction_destinations,
            transport_rx: Arc::new(Mutex::new(transport_rx)),
        }
    }

    /// Send a request through a client transaction
    pub async fn send_request(&self, transaction_id: &TransactionKey) -> Result<()> {
        // We need to get the transaction and clone only when needed
        let mut locked_txs = self.client_transactions.lock().await;
        
        // Check if transaction exists
        if !locked_txs.contains_key(transaction_id) {
            return Err(Error::transaction_not_found(transaction_id.clone(), "send_request - transaction not found"));
        }
        
        // Get a reference to the transaction to determine its type
        let tx = locked_txs.get_mut(transaction_id).unwrap();
        
        // Use the TransactionExt trait to safely downcast
        use crate::client::TransactionExt;
        
        if let Some(client_tx) = tx.as_client_transaction() {
            client_tx.initiate().await
        } else {
            Err(Error::Other("Failed to downcast to client transaction".to_string()))
        }
    }

    /// Send a response through a server transaction
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> Result<()> {
        // We need to get the transaction and clone only when needed
        let mut locked_txs = self.server_transactions.lock().await;
        
        // Check if transaction exists
        if !locked_txs.contains_key(transaction_id) {
            return Err(Error::transaction_not_found(transaction_id.clone(), "send_response - transaction not found"));
        }
        
        // Get a reference to the transaction to determine its type
        let tx = locked_txs.get_mut(transaction_id).unwrap();
        
        // Use the TransactionExt trait to safely downcast
        use crate::server::TransactionExt;
        
        if let Some(server_tx) = tx.as_server_transaction() {
            server_tx.send_response(response).await
        } else {
            Err(Error::Other("Failed to downcast to server transaction".to_string()))
        }
    }

    /// Check if a transaction exists
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

    /// Get the state of a transaction
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

    /// Get the transaction type (kind)
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

    /// Get all active transaction IDs (keys)
    pub async fn active_transactions(&self) -> (Vec<TransactionKey>, Vec<TransactionKey>) {
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

    /// Subscribe to transaction events
    pub fn subscribe(&self) -> mpsc::Receiver<TransactionEvent> {
        let (tx, rx) = mpsc::channel(100); // Consider configurable capacity

        // Add the sender asynchronously
         tokio::spawn({
             let event_subscribers = self.event_subscribers.clone();
             async move {
                 event_subscribers.lock().await.push(tx);
             }
         });

        rx
    }

    /// Shutdown the transaction manager
    pub async fn shutdown(&self) {
        {
             let mut running = self.running.lock().await;
             *running = false;
        } // Release lock

        // Clear all transactions
        self.client_transactions.lock().await.clear();
        self.server_transactions.lock().await.clear();
        self.transaction_destinations.lock().await.clear();

        debug!("Transaction manager shutdown");
    }

    /// Broadcast events to primary and all subscriber channels
    async fn broadcast_event(
        event: TransactionEvent,
        primary_tx: &mpsc::Sender<TransactionEvent>,
        subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
        manager: Option<TransactionManager>, // Optional manager for processing termination events
    ) {
        // If this is a termination event, handle it specially
        if let TransactionEvent::TransactionTerminated { transaction_id } = &event {
            // Log the termination event
            debug!(%transaction_id, "Transaction termination event received in broadcast_event");
            
            // We need to send to the primary channel so the manager can process it
            if let Err(e) = primary_tx.send(event.clone()).await {
                error!("Failed to send termination event to primary channel: {}", e);
            }
            
            // If manager is provided, process the termination event
            if let Some(manager) = manager {
                debug!(%transaction_id, "Processing termination event with manager");
                manager.process_transaction_terminated(transaction_id).await;
            }
            
            // Don't forward to subscribers - this is an internal event
            return;
        }
        
        // Send to primary channel
        if let Err(e) = primary_tx.send(event.clone()).await {
            warn!("Failed to send event to primary channel: {}", e);
        }
        
        // Send to all subscribers
        let subscriber_channels = subscribers.lock().await.clone();
        for subscriber in subscriber_channels {
            if let Err(e) = subscriber.send(event.clone()).await {
                warn!("Failed to send event to subscriber: {}", e);
                // TODO: Consider subscriber cleanup
            }
        }
    }

    /// Handle transaction termination event and clean up terminated transactions
    async fn process_transaction_terminated(&self, transaction_id: &TransactionKey) {
        debug!(%transaction_id, "Processing transaction termination");
        
        // Try to remove from client transactions
        {
            let mut client_txs = self.client_transactions.lock().await;
            if client_txs.remove(transaction_id).is_some() {
                debug!(%transaction_id, "Removed terminated client transaction");
                // Also remove from destinations map
                let mut destinations = self.transaction_destinations.lock().await;
                destinations.remove(transaction_id);
                return;
            }
        }
        
        // Try to remove from server transactions
        {
            let mut server_txs = self.server_transactions.lock().await;
            if server_txs.remove(transaction_id).is_some() {
                debug!(%transaction_id, "Removed terminated server transaction");
                return;
            }
        }
        
        debug!(%transaction_id, "Transaction not found for termination - may have been already removed");
    }

    /// Start the message processing loop for handling incoming transport events
    fn start_message_loop(&self) {
        let transport_arc = self.transport.clone();
        let client_transactions = self.client_transactions.clone();
        let server_transactions = self.server_transactions.clone();
        let events_tx = self.events_tx.clone();
        let transport_rx = self.transport_rx.clone();
        let event_subscribers = self.event_subscribers.clone();
        let running = self.running.clone();
        let manager_arc = self.clone();

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
                        if let Err(e) = handle_transport_message(
                            message_event,
                            &transport_arc,
                            &client_transactions,
                            &server_transactions,
                            &events_tx,
                            &event_subscribers,
                            &manager_arc,
                        ).await {
                            error!("Error handling transport message: {}", e);
                        }
                    }
                    Some(transaction_event) = internal_rx.recv() => {
                        // Handle transaction events, particularly termination events
                        Self::broadcast_event(
                            transaction_event, 
                            &events_tx, 
                            &event_subscribers,
                            Some(manager_arc.clone()),
                        ).await;
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

    /// Create a client transaction for sending a SIP request
    /// The caller is responsible for calling send_request() to initiate the transaction.
    pub async fn create_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        debug!(method = %request.method(), %destination, "Creating client transaction");
        
        // Check if we've already registered a transaction with that branch
        let branch = match request.first_via() {
            Some(via) => {
                match via.branch() {
                    Some(b) if b.starts_with(RFC3261_BRANCH_MAGIC_COOKIE) => b.to_string(),
                    _ => generate_branch(),
                }
            },
            None => generate_branch(),
        };
        
        let local_addr = self.transport.local_addr()
            .map_err(|e| Error::transport_error(e, "Failed to get local address"))?;
            
        // Create request with proper Via header
        let mut modified_request = request.clone();
        
        // Now add our Via header to the top of the stack
        let local_host = local_addr.ip().to_string();
        let local_port = local_addr.port();
        
        // Create a new Via header and insert it
        let mut via_params = vec![Param::branch(&branch)];
        
        // Add other params like rport if needed
        // Example: via_params.push(Param::other("rport".to_string(), None));
        
        let via = Via::new("SIP", "2.0", "UDP", &local_host, Some(local_port), via_params)
            .map_err(Error::SipCoreError)?;
        
        modified_request.headers.insert(0, TypedHeader::Via(via));
        
        // Create transaction key/ID
        let key = transaction_key_from_message(&Message::Request(modified_request.clone()))
            .ok_or_else(|| Error::Other("Could not determine transaction key".into()))?;
            
        // Store the destination so it can be used for ACKs outside the transaction
        {
            let mut dest_map = self.transaction_destinations.lock().await;
            dest_map.insert(key.clone(), destination);
        }
        
        // Create transaction based on request type
        if modified_request.method() == Method::Invite {
            // Create an INVITE client transaction
            let transaction = ClientInviteTransaction::new(
                key.clone(),
                modified_request,
                destination,
                self.transport.clone(),
                self.events_tx.clone(),
                Some(self.timer_settings.clone()),
            )?;
            
            // Store the transaction
            let mut client_txs = self.client_transactions.lock().await;
            client_txs.insert(key.clone(), Box::new(transaction));
        } else {
            // Create a non-INVITE client transaction
            let transaction = ClientNonInviteTransaction::new(
                key.clone(),
                modified_request,
                destination,
                self.transport.clone(),
                self.events_tx.clone(),
                Some(self.timer_settings.clone()),
            )?;
            
            // Store the transaction
            let mut client_txs = self.client_transactions.lock().await;
            client_txs.insert(key.clone(), Box::new(transaction));
        }
        
        debug!(id=%key, "Created client transaction");
        
        Ok(key)
    }

    /// Send an ACK for a 2xx response
    pub async fn send_2xx_ack(
        &self,
        final_response: &Response,
    ) -> Result<()> {
        // Get Request-URI from Contact or To
        let request_uri = match final_response.header(&HeaderName::Contact) { 
            Some(contact) => {
                if let TypedHeader::Contact(contact_val) = contact {
                    contact_val.addresses().next().map(|a| a.uri.clone())
                } else { None }
            },
            None => None,
        }.or_else(|| {
            final_response.header(&HeaderName::To)
                .and_then(|to| if let TypedHeader::To(to_val) = to { 
                    Some(to_val.address().uri.clone()) 
                } else { None })
        }).ok_or_else(|| Error::Other("Cannot determine Request-URI for ACK from response".into()))?;

        let mut ack_builder = RequestBuilder::new(Method::Ack, &request_uri.to_string())?;

        if let Some(via) = final_response.first_via() {
            ack_builder = ack_builder.header(TypedHeader::Via(via.clone())); // Wrap
        } else {
            return Err(Error::Other("Cannot determine Via header for ACK from response".into()));
        }
        
        if let Some(from) = final_response.header(&HeaderName::From) {
            if let TypedHeader::From(from_val) = from {
                ack_builder = ack_builder.header(TypedHeader::From(from_val.clone())); // Wrap
            } else {
                return Err(Error::Other("Missing or invalid From header in response for ACK".into()));
            }
        } else {
            return Err(Error::Other("Missing From header in response for ACK".into()));
        }
        
        if let Some(to) = final_response.header(&HeaderName::To) {
            if let TypedHeader::To(to_val) = to {
                ack_builder = ack_builder.header(TypedHeader::To(to_val.clone())); // Wrap
            } else {
                return Err(Error::Other("Missing or invalid To header in response for ACK".into()));
            }
        } else {
            return Err(Error::Other("Missing To header in response for ACK".into()));
        }
        
        if let Some(call_id) = final_response.header(&HeaderName::CallId) {
            if let TypedHeader::CallId(call_id_val) = call_id {
                ack_builder = ack_builder.header(TypedHeader::CallId(call_id_val.clone())); // Wrap
            } else {
                return Err(Error::Other("Missing or invalid Call-ID header in response for ACK".into()));
            }
        } else {
            return Err(Error::Other("Missing Call-ID header in response for ACK".into()));
        }
        
        // Use utils::extract_cseq on the message
        if let Some((seq, _)) = extract_cseq(&Message::Response(final_response.clone())) {
            ack_builder = ack_builder.header(TypedHeader::CSeq(CSeq::new(seq, Method::Ack))); // Wrap
        } else {
            return Err(Error::Other("Missing or invalid CSeq header in response for ACK".into()));
        }

        ack_builder = ack_builder.header(TypedHeader::MaxForwards(MaxForwards::new(70))); // Wrap
        ack_builder = ack_builder.header(TypedHeader::ContentLength(ContentLength::new(0))); // Wrap

        let ack_request = ack_builder.build();

        let destination = handlers::determine_ack_destination(final_response).await
            .ok_or_else(|| Error::Other("Could not determine destination for ACK".into()))?;

        info!(%destination, "Sending ACK for 2xx response");

        self.transport.send_message(
            Message::Request(ack_request),
            destination
        ).await.map_err(|e| Error::transport_error(e, "Failed to send ACK for non-2xx response"))
    }

    // Test-only method to get server transactions for debugging
    #[cfg(test)]
    pub async fn get_server_transactions_for_test(&self) -> Vec<String> {
        let transactions = self.server_transactions.lock().await;
        transactions.keys().map(|k| k.to_string()).collect()
    }

    /// Create a server transaction from a request for testing purposes
    #[cfg(test)]
    pub async fn create_server_transaction(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source_addr: SocketAddr,
    ) -> Result<()> {
        use crate::server::{ServerInviteTransaction, ServerNonInviteTransaction};
        
        // Get the server transactions map
        let mut server_txs = self.server_transactions.lock().await;
        
        // Create server transaction based on request type
        let server_tx: Box<dyn crate::server::ServerTransaction + Send> = if request.method() == Method::Invite {
            Box::new(ServerInviteTransaction::new(
                transaction_id.clone(),
                request.clone(),
                source_addr,
                self.transport.clone(),
                self.events_tx.clone(),
                Some(self.timer_settings.clone()),
            )?)
        } else {
            Box::new(ServerNonInviteTransaction::new(
                transaction_id.clone(),
                request.clone(),
                source_addr,
                self.transport.clone(),
                self.events_tx.clone(),
                Some(self.timer_settings.clone()),
            )?)
        };
        
        // Store transaction
        server_txs.insert(transaction_id.clone(), server_tx);
        
        // Store destination
        let mut destinations = self.transaction_destinations.lock().await;
        destinations.insert(transaction_id, source_addr);
        
        Ok(())
    }
}

impl fmt::Debug for TransactionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Avoid trying to print the Mutex contents directly or requiring Debug on contents
        f.debug_struct("TransactionManager")
            .field("transport", &"Arc<dyn Transport>")
            .field("client_transactions", &"Arc<Mutex<HashMap<...>>>") // Indicate map exists
            .field("server_transactions", &"Arc<Mutex<HashMap<...>>>")
            .field("transaction_destinations", &"Arc<Mutex<HashMap<...>>>")
            .field("events_tx", &self.events_tx) // Sender might be Debug
            .field("event_subscribers", &"Arc<Mutex<Vec<Sender>>>")
            .field("transport_rx", &"Arc<Mutex<Receiver>>")
            .field("running", &self.running)
            .field("timer_settings", &self.timer_settings)
            .field("timer_manager", &"Arc<TimerManager>")
            .field("timer_factory", &"TimerFactory")
            .finish()
    }
} 