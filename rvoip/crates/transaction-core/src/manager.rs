use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::str::FromStr;
use std::pin::Pin;

use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

// Use prelude for common types
use rvoip_sip_core::prelude::*;
// Use specific types not in prelude if needed
use rvoip_sip_core::{Host, TypedHeader};

use rvoip_sip_transport::{Transport, TransportEvent};

// Update internal imports based on actual structure
use crate::error::{self, Error, Result}; // Assuming Result is defined in error.rs
use crate::transaction::{ // Updated imports
    Transaction, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    client::{self, ClientTransaction, ClientInviteTransaction, ClientNonInviteTransaction},
    server::{self, ServerTransaction, ServerInviteTransaction, ServerNonInviteTransaction},
};
// Remove redundant crate::error::Result import if already imported above
// Remove crate::context::TransactionContext if not used
use crate::utils; // Keep utils import
use rvoip_sip_core::parse_message; // Import explicitly if needed for stray messages
use rvoip_sip_core::types::builder::ViaBuilder; // Import ViaBuilder

/// Transaction timer data
struct TransactionTimer {
    /// Transaction ID
    transaction_id: String,
    /// When the timer should fire
    expiry: Instant,
}

// Type aliases without Sync requirement
type BoxedTransaction = Box<dyn Transaction + Send>;
type BoxedServerTransaction = Box<dyn ServerTransaction + Send>;
type BoxedClientTransaction = Box<dyn ClientTransaction + Send>;

/// Manages SIP transactions
pub struct TransactionManager {
    /// Transport to use for messages
    transport: Arc<dyn Transport>,
    /// Active client transactions
    client_transactions: Arc<Mutex<HashMap<TransactionKey, BoxedClientTransaction>>>, // Use TransactionKey
    /// Active server transactions
    server_transactions: Arc<Mutex<HashMap<TransactionKey, BoxedServerTransaction>>>, // Use TransactionKey
    /// Transaction destinations - maps transaction IDs to their destinations
    transaction_destinations: Arc<Mutex<HashMap<TransactionKey, SocketAddr>>>, // Use TransactionKey
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
        // manager.start_timer_loop(); // Consider if timer logic belongs here or within transactions

        Ok((manager, events_rx))
    }

    /// Create a dummy transaction manager
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

    // Helper to broadcast events
    async fn broadcast_event(
        event: TransactionEvent,
        primary_tx: &mpsc::Sender<TransactionEvent>,
        subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
    ) {
        if let Err(e) = primary_tx.send(event.clone()).await {
            error!("Failed to send event to primary channel: {}", e);
        }

        let mut subs = subscribers.lock().await;
        // Clone senders before spawning tasks to avoid lifetime issues
        let current_subs: Vec<_> = subs.iter().cloned().collect();
        subs.clear(); // Clear the original list to repopulate with active ones

        for tx in current_subs {
            let event_clone = event.clone();
            let tx_clone = tx.clone(); // Clone sender for the task
            tokio::spawn(async move {
                if tx_clone.send(event_clone).await.is_err() {
                     error!("Failed to send event to subscriber, removing.");
                     // Don't re-add tx_clone to the list
                 } else {
                    // Need to re-add the sender back to the shared list, requires locking again.
                    // This approach is complex. A simpler way is to clean up periodically
                    // or use a broadcast channel if exact delivery isn't critical.
                    // For now, we just won't remove failed subscribers efficiently here.
                 }
             });
             // Re-add the original sender if it's likely still valid (optimistic)
             // A better approach is needed for robust subscriber management.
             if !tx.is_closed() {
                subs.push(tx);
             }
        }
    }


    /// Start processing incoming transport messages
    fn start_message_loop(&self) {
        let transport_arc = self.transport.clone();
        let client_transactions = self.client_transactions.clone();
        let server_transactions = self.server_transactions.clone();
        let events_tx = self.events_tx.clone();
        let transport_rx = self.transport_rx.clone();
        let event_subscribers = self.event_subscribers.clone();
        let running = self.running.clone();

        tokio::spawn(async move {
            debug!("Starting transaction message loop");

            while *running.lock().await {
                let event = match transport_rx.lock().await.recv().await {
                    Some(event) => event,
                    None => {
                        debug!("Transport channel closed, stopping message loop");
                        break;
                    }
                };

                match event {
                    TransportEvent::MessageReceived { message, source, destination } => {
                        debug!(source = %source, ?message, "Received message");

                        // Use the moved function from utils
                         let key_result = utils::transaction_key_from_message(&message);

                         match key_result {
                             Ok(key) => {
                                 match message {
                                     Message::Request(request) => {
                                         if let Err(e) = Self::process_request(
                                             key,
                                             request,
                                             source,
                                             destination, // Assuming destination is local address
                                             &transport_arc, // Pass Arc
                                             &server_transactions,
                                             &events_tx,
                                             &event_subscribers,
                                         ).await {
                                             error!(error = %e, source = %source, "Failed to process request");
                                         }
                                     }
                                     Message::Response(response) => {
                                         if let Err(e) = Self::process_response(
                                             key,
                                             response,
                                             source,
                                             &client_transactions,
                                             &events_tx,
                                             &event_subscribers,
                                         ).await {
                                             error!(error = %e, source = %source, "Failed to process response");
                                         }
                                     }
                                 }
                             },
                             Err(e) => {
                                 error!(error = %e, ?message, "Failed to extract transaction key");
                                 // Handle stray message - parse again if needed, or use raw bytes
                                 // Maybe the transport event should include raw bytes?
                                 // Assuming message is still valid for broadcasting:
                                 let stray_event = match message {
                                     Message::Request(req) => TransactionEvent::StrayRequest { request: req, source },
                                     Message::Response(res) => TransactionEvent::StrayResponse { response: res, source },
                                 };
                                 Self::broadcast_event(
                                     stray_event,
                                     &events_tx,
                                     &event_subscribers
                                 ).await;
                                 // Also broadcast generic error
                                  Self::broadcast_event(
                                      TransactionEvent::Error { error: e.to_string(), transaction_id: None },
                                      &events_tx,
                                      &event_subscribers
                                  ).await;
                             }
                         }
                    }
                    TransportEvent::Error { error } => {
                        warn!(error = %error, "Transport error");
                        Self::broadcast_event(
                            TransactionEvent::Error { error, transaction_id: None },
                            &events_tx,
                            &event_subscribers
                        ).await;
                    }
                    TransportEvent::Closed => {
                        info!("Transport closed");
                        break;
                    }
                }
            }

            debug!("Transaction message loop stopped");
        });
    }

    // TODO: Re-evaluate if timer loop is needed at manager level
    // Timers T1, T2, T4 (INVITE client), Timer A-K (non-INVITE client)
    // Timers G, H, I (INVITE server), Timer J (non-INVITE server)
    // These are often handled *within* the specific transaction state machines.

    /*
    fn start_timer_loop(&self) {
        // ... (Implementation if needed) ...
    }
    */

    /// Process an incoming request (addressed to us)
    async fn process_request(
        key: TransactionKey,
        request: Request,
        source: SocketAddr,
        _local_addr: SocketAddr, // Our local address the message arrived on
        transport: &Arc<dyn Transport>, // Use reference to Arc
        server_transactions: &Arc<Mutex<HashMap<TransactionKey, BoxedServerTransaction>>>,
        manager_events_tx: &mpsc::Sender<TransactionEvent>,
        manager_event_subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
    ) -> Result<()> {
        let mut transactions = server_transactions.lock().await;

        // Check if transaction already exists
        if let Some(tx) = transactions.get_mut(&key) {
             debug!(%key, "Request matched existing server transaction");
             // Pass the request to the existing transaction state machine
             // The transaction itself will handle retransmissions, state changes, etc.
             if let Err(e) = tx.process_request(request).await {
                error!(error = %e, %key, "Error processing request in existing server transaction");
                 Self::broadcast_event(
                    TransactionEvent::Error { error: e.to_string(), transaction_id: Some(key.clone()) },
                    manager_events_tx,
                    manager_event_subscribers
                ).await;
                return Err(e);
             }
             // If successful, potentially broadcast state change event if needed
             return Ok(());
        }

        // --- No existing transaction, create a new one ---

        // ACK is special: It never creates a transaction. It matches an existing INVITE server transaction.
        // If it doesn't match, it's considered "stray" and passed up. (Handled by key matching now)
        if request.method() == Method::Ack {
            warn!(%key, source = %source, "Received stray ACK (no matching INVITE server transaction)");
             Self::broadcast_event(
                TransactionEvent::StrayAck { request, source }, // Define this event type
                manager_events_tx,
                manager_event_subscribers
            ).await;
            return Ok(());
        }

        // CANCEL is also special: It matches an existing INVITE server transaction by key.
        // If it doesn't match, it should get a 481 response.
        if request.method() == Method::Cancel {
             warn!(%key, source = %source, "Received CANCEL for non-existent transaction");
             // Respond 481 Transaction Does Not Exist
             // CANCEL shares same branch as INVITE, but TU handles response
             let response = ResponseBuilder::new(StatusCode::CallOrTransactionDoesNotExist)
                .copy_essential_headers(&request)? // Use Result from copy_essential_headers
                .build(); // Assuming build is infallible or handled
             if let Err(e) = transport.send_message(Message::Response(response), source).await {
                error!(error = %e, "Failed to send 481 for CANCEL");
             }
             // Optionally notify TU about stray cancel
             Self::broadcast_event(
                TransactionEvent::StrayCancel { request, source }, // Define this event type
                manager_events_tx,
                manager_event_subscribers
            ).await;
             return Ok(());
        }


        debug!(%key, method = %request.method(), "Creating new server transaction");

        let tx_result = if request.method() == Method::Invite {
            ServerInviteTransaction::new(
                key.clone(),
                request.clone(), // Clone request for transaction state
                source,
                transport.clone(),
                manager_events_tx.clone(), // Pass correct sender
            ).map(|tx| Box::new(tx) as BoxedServerTransaction)
        } else {
            ServerNonInviteTransaction::new(
                key.clone(),
                request.clone(),
                source,
                transport.clone(),
                manager_events_tx.clone(), // Pass correct sender
            ).map(|tx| Box::new(tx) as BoxedServerTransaction)
        };

        match tx_result {
            Ok(tx) => {
                let transaction_id = tx.id().clone(); // Use the key as ID
                debug!(%transaction_id, "Created new server transaction");

                 // Add transaction to collection BEFORE passing up the request
                transactions.insert(transaction_id.clone(), tx);

                // Notify TU (e.g., Session layer) about the *new* request for this transaction
                 Self::broadcast_event(
                    TransactionEvent::NewRequest { transaction_id, request, source }, // Define this event
                    manager_events_tx,
                    manager_event_subscribers
                ).await;

                Ok(())
            }
            Err(e) => {
                error!(error = %e, "Failed to create server transaction");
                 Self::broadcast_event(
                    TransactionEvent::Error { error: e.to_string(), transaction_id: Some(key) },
                    manager_events_tx,
                    manager_event_subscribers
                ).await;
                Err(e)
            }
        }
    }


    /// Process an incoming response (addressed to us)
    async fn process_response(
        key: TransactionKey,
        response: Response,
        source: SocketAddr,
        client_transactions: &Arc<Mutex<HashMap<TransactionKey, BoxedClientTransaction>>>,
        events_tx: &mpsc::Sender<TransactionEvent>,
        event_subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
    ) -> Result<()> {
        let mut transactions = client_transactions.lock().await;

        if let Some(tx) = transactions.get_mut(&key) {
             debug!(%key, status = %response.status(), "Response matched existing client transaction");

             // Send response to the transaction state machine
             if let Err(e) = tx.process_response(response).await {
                 error!(error = %e, %key, "Error processing response in client transaction");
                 Self::broadcast_event(
                     TransactionEvent::Error { error: e.to_string(), transaction_id: Some(key.clone()) },
                     events_tx,
                     event_subscribers
                 ).await;
                 return Err(e);
             }
             // Transaction state machine handles passing response to TU via events_tx
             Ok(())

        } else {
            // No matching client transaction found for this response
             warn!(%key, status = %response.status(), source = %source, "Received stray response");
             Self::broadcast_event(
                 TransactionEvent::StrayResponse { response, source }, // Define this event
                 events_tx,
                 event_subscribers
             ).await;
             Ok(())
        }
    }

    /// Create and start a client transaction for sending a request
    pub async fn create_client_transaction(
        &self,
        mut request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        debug!(method = %request.method(), %destination, "Creating client transaction");

        let branch = match request.first_via().and_then(|v| v.branch()) {
            Some(b) if !b.starts_with(RFC3261_BRANCH_MAGIC_COOKIE) => utils::generate_branch(),
            Some(b) => b.to_string(),
            None => utils::generate_branch(),
        };

        let local_addr = self.transport.local_addr()
            .map_err(|e| Error::TransportError(e.to_string()))?;
        // Use Via::new
        let via = Via::new("SIP", "2.0", "UDP", &local_addr.ip().to_string(), Some(local_addr.port()),
            vec![Param::branch(&branch)]).map_err(Error::SipCoreError)?;

        request.headers.insert(0, TypedHeader::Via(via));

        // Use utils function for key
        let key = utils::transaction_key_from_message(&Message::Request(request.clone()))?;

         let tx_result = if request.method() == Method::Invite {
             ClientInviteTransaction::new(
                 key.clone(),
                 request,
                 destination,
                 self.transport.clone(),
                 self.events_tx.clone(), // Use self.
             ).map(|tx| Box::new(tx) as BoxedClientTransaction)
         } else {
             ClientNonInviteTransaction::new(
                 key.clone(),
                 request,
                 destination,
                 self.transport.clone(),
                 self.events_tx.clone(), // Use self.
             ).map(|tx| Box::new(tx) as BoxedClientTransaction)
         };

        match tx_result {
            Ok(tx) => {
                let transaction_id = tx.id().clone();
                debug!("Created client transaction with ID: {}", transaction_id);
                let mut client_txs = self.client_transactions.lock().await;
                client_txs.insert(transaction_id.clone(), tx);
                let mut destinations = self.transaction_destinations.lock().await;
                destinations.insert(transaction_id.clone(), destination);
                debug!(%destination, %transaction_id, "Stored destination for transaction");
                Ok(transaction_id)
            }
            Err(e) => {
                error!(error = %e, "Failed to create client transaction");
                 Self::broadcast_event(
                    TransactionEvent::Error { error: e.to_string(), transaction_id: Some(key) },
                    &self.events_tx,
                    &self.event_subscribers
                ).await;
                Err(e)
            }
        }
    }

    /// Send the initial request for a client transaction
    pub async fn send_request(&self, transaction_id: &TransactionKey) -> Result<()> {
        debug!(%transaction_id, "Initiating send for client transaction");

        let mut transactions = self.client_transactions.lock().await;
        if let Some(transaction) = transactions.get_mut(transaction_id) {
            // The transaction state machine handles the actual sending
             if let Err(e) = transaction.initiate().await { // Assuming an initiate method
                 error!(error = %e, %transaction_id, "Failed to initiate send");
                  // Notify TU about the error
                  Self::broadcast_event(
                    TransactionEvent::Error { error: e.to_string(), transaction_id: Some(transaction_id.clone()) },
                    &self.events_tx,
                    &self.event_subscribers
                ).await;
                 Err(e)
             } else {
                 debug!(%transaction_id, "Transaction initiated successfully");
                 Ok(())
             }
        } else {
            error!(%transaction_id, "Transaction not found for sending request");
            Err(Error::TransactionNotFound(transaction_id.clone()))
        }
    }


    /// Send a response via the corresponding server transaction
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> Result<()> {
        let mut transactions = self.server_transactions.lock().await;

        if let Some(tx) = transactions.get_mut(transaction_id) {
            debug!(%transaction_id, status = %response.status(), "Sending response via server transaction");
            // The transaction state machine handles sending, retransmissions, etc.
            if let Err(e) = tx.send_response(response).await {
                 error!(error = %e, %transaction_id, "Failed to send response via transaction");
                 Self::broadcast_event(
                    TransactionEvent::Error { error: e.to_string(), transaction_id: Some(transaction_id.clone()) },
                    &self.events_tx,
                    &self.event_subscribers
                ).await;
                 Err(e)
             } else {
                Ok(())
             }
        } else {
            error!(%transaction_id, "Transaction not found for sending response");
            Err(Error::TransactionNotFound(transaction_id.clone()))
        }
    }

    /// Get the transaction state
    pub async fn transaction_state(&self, transaction_id: &TransactionKey) -> Result<TransactionState> {
        let client_txs = self.client_transactions.lock().await;
        if let Some(tx) = client_txs.get(transaction_id) {
            return Ok(tx.state());
        }

        let server_txs = self.server_transactions.lock().await;
        if let Some(tx) = server_txs.get(transaction_id) {
            return Ok(tx.state());
        }

        Err(Error::TransactionNotFound(transaction_id.clone()))
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

        Err(Error::TransactionNotFound(transaction_id.clone()))
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

    /// Shutdown the transaction manager
    pub async fn shutdown(&self) {
        {
             let mut running = self.running.lock().await;
             *running = false;
        } // Release lock

        // TODO: Gracefully terminate transactions?
        // Currently just clears them.

        self.client_transactions.lock().await.clear();
        self.server_transactions.lock().await.clear();
        self.transaction_destinations.lock().await.clear();
        // Close event channels? Depends on desired shutdown behavior.

        debug!("Transaction manager shutdown");
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

    /// Send an ACK for a 2xx response
    pub async fn send_2xx_ack(
        &self,
        final_response: &Response,
    ) -> Result<()> {

         // Get Request-URI from Contact or To
         let request_uri = match final_response.header::<Contact>() { 
             Some(contact) => contact.addresses().next().map(|a| a.uri.clone()),
             None => None,
         }.or_else(|| final_response.header::<To>().map(|t| t.address().uri.clone())) 
          .ok_or_else(|| Error::Other("Cannot determine Request-URI for ACK from response".into()))?;

         let mut ack_builder = RequestBuilder::new(Method::Ack, &request_uri.to_string())?;

         if let Some(via) = final_response.first_via() {
             ack_builder = ack_builder.header(TypedHeader::Via(via.clone())); // Wrap
         } else {
             return Err(Error::Other("Cannot determine Via header for ACK from response".into()));
         }
         if let Some(from) = final_response.from() {
            ack_builder = ack_builder.header(TypedHeader::From(from.clone())); // Wrap
         } else {
             return Err(Error::Other("Missing From header in response for ACK".into()));
         }
         if let Some(to) = final_response.to() {
             ack_builder = ack_builder.header(TypedHeader::To(to.clone())); // Wrap
         } else {
             return Err(Error::Other("Missing To header in response for ACK".into()));
         }
         if let Some(call_id) = final_response.call_id() {
             ack_builder = ack_builder.header(TypedHeader::CallId(call_id.clone())); // Wrap
         } else {
             return Err(Error::Other("Missing Call-ID header in response for ACK".into()));
         }
         // Use utils::extract_cseq on the message
         if let Some((seq, _)) = utils::extract_cseq(&Message::Response(final_response.clone())) {
             ack_builder = ack_builder.header(TypedHeader::CSeq(CSeq::new(seq, Method::Ack))); // Wrap
         } else {
             return Err(Error::Other("Missing or invalid CSeq header in response for ACK".into()));
         }

         ack_builder = ack_builder.header(TypedHeader::MaxForwards(MaxForwards::new(70))); // Wrap
         ack_builder = ack_builder.header(TypedHeader::ContentLength(ContentLength::new(0))); // Wrap

         let ack_request = ack_builder.build();

         let destination = Self::determine_ack_destination(final_response).await
             .ok_or_else(|| Error::Other("Could not determine destination for ACK".into()))?;

         info!(%destination, "Sending ACK for 2xx response");

         self.transport.send_message(
             Message::Request(ack_request),
             destination
         ).await.map_err(|e| Error::TransportError(e.to_string()))

    }

     // Helper to determine ACK destination
     async fn determine_ack_destination(response: &Response) -> Option<SocketAddr> {
         if let Some(contact) = response.header::<Contact>() {
             if let Some(addr) = contact.addresses().next() {
                  if let Some(dest) = Self::resolve_uri_to_socketaddr(&addr.uri).await {
                      return Some(dest);
                  }
             }
         }
         if let Some(via) = response.first_via() {
              if let (Some(received_ip_str), rport_opt) = (via.received(), via.rport()) {
                  // Use IpAddr::from_str
                  if let Ok(ip) = IpAddr::from_str(received_ip_str) {
                      // Handle Option<u16> for rport using ok_or or if let
                      if let Some(port) = rport_opt {
                         let dest = SocketAddr::new(ip, port);
                         return Some(dest);
                      } else {
                          warn!("Via had received but no rport");
                      }
                  } else {
                      warn!(ip=%received_ip_str, "Failed to parse received IP in Via");
                  }
              }
              // Fallback to Via host/port
              let host_str = via.sent_by_host().unwrap_or("localhost");
              let port = via.sent_by_port().unwrap_or(5060);
              
              // Create a proper Host enum
              let host = if let Ok(ip) = IpAddr::from_str(host_str) {
                  Host::Address(ip)
              } else {
                  Host::Domain(host_str.to_string())
              };
              
              if let Some(dest) = Self::resolve_host_to_socketaddr(&host, port).await {
                  return Some(dest);
              }
         }
         None
     }

     // Helper to resolve URI host to SocketAddr
     async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr> {
         let port = uri.port.unwrap_or(5060);
         Self::resolve_host_to_socketaddr(&uri.host, port).await
     }

     // Helper to resolve Host enum to SocketAddr
      async fn resolve_host_to_socketaddr(host: &Host, port: u16) -> Option<SocketAddr> {
          match host {
              Host::Address(ip) => Some(SocketAddr::new(*ip, port)),
              Host::Domain(domain) => {
                  if let Ok(ip) = IpAddr::from_str(domain) { // Use FromStr
                      return Some(SocketAddr::new(ip, port));
                  }
                  match tokio::net::lookup_host(format!("{}:{}", domain, port)).await {
                      Ok(mut addrs) => addrs.next(),
                      Err(e) => {
                          error!(error = %e, domain = %domain, "DNS lookup failed for ACK destination");
                          None
                      }
                  }
              }
          }
      }

}

// ResponseBuilderExt trait - Use specific accessors and wrap headers
trait ResponseBuilderExt {
    fn copy_essential_headers(self, request: &Request) -> Result<Self> where Self: Sized;
}

impl ResponseBuilderExt for ResponseBuilder {
     fn copy_essential_headers(mut self, request: &Request) -> Result<Self> {
        if let Some(via) = request.first_via() {
             self = self.header(TypedHeader::Via(via.clone()));
         }
         if let Some(to) = request.header::<To>() {
             self = self.header(TypedHeader::To(to.clone()));
         }
         if let Some(from) = request.header::<From>() {
             self = self.header(TypedHeader::From(from.clone()));
         }
         if let Some(call_id) = request.header::<CallId>() {
             self = self.header(TypedHeader::CallId(call_id.clone()));
         }
         if let Some(cseq) = request.header::<CSeq>() {
             self = self.header(TypedHeader::CSeq(cseq.clone()));
         }
         self = self.header(TypedHeader::ContentLength(ContentLength::new(0)));
         Ok(self) // Return Result
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
            .finish()
    }
}

// Define RFC3261 Branch magic cookie
const RFC3261_BRANCH_MAGIC_COOKIE: &str = "z9hG4bK";
