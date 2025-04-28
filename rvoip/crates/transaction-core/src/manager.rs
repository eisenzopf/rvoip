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

// Use prelude for common types
use rvoip_sip_core::prelude::*;
// Use specific types not in prelude if needed
use rvoip_sip_core::{Host, TypedHeader};

use rvoip_sip_transport::{Transport, TransportEvent};

// Update internal imports based on actual structure
use crate::error::{self, Error, Result}; // Assuming Result is defined in error.rs
use crate::transaction::{ // Assuming these are defined in transaction/mod.rs or lib.rs
    Transaction, TransactionState, TransactionType, TransactionEvent,
    TransactionId, TransactionKey, TransactionKind, // If these are distinct types
    client::{self, ClientTransaction, ClientInviteTransaction, ClientNonInviteTransaction},
    server::{self, ServerTransaction, ServerInviteTransaction, ServerNonInviteTransaction},
};
// Remove redundant crate::error::Result import if already imported above
// Remove crate::context::TransactionContext if not used
use crate::utils; // Keep utils import

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
        // Use retain_mut for efficient removal of closed channels
        subs.retain_mut(|tx| {
            let event_clone = event.clone();
            tokio::spawn(async move {
                if tx.send(event_clone).await.is_err() {
                     error!("Failed to send event to subscriber, removing.");
                     // Signal to retain to remove this sender
                     false
                 } else {
                     true
                 }
             });
             !tx.is_closed() // Keep if not closed (async task will handle actual send)
        });
    }


    /// Start processing incoming transport messages
    fn start_message_loop(&self) {
        let transport_arc = self.transport.clone(); // Keep Arc for transport
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

                        // Determine transaction key
                         let key = match TransactionKey::from_message(&message) {
                            Ok(key) => key,
                            Err(e) => {
                                error!(error = %e, ?message, "Failed to extract transaction key");
                                // Send error event or handle appropriately
                                Self::broadcast_event(
                                    TransactionEvent::Error { error: e.to_string(), transaction_id: None },
                                    &events_tx,
                                    &event_subscribers
                                ).await;
                                continue;
                            }
                        };

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
        events_tx: &mpsc::Sender<TransactionEvent>,
        event_subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
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
                    events_tx,
                    event_subscribers
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
                events_tx,
                event_subscribers
            ).await;
            return Ok(());
        }

        // CANCEL is also special: It matches an existing INVITE server transaction by key.
        // If it doesn't match, it should get a 481 response.
        if request.method() == Method::Cancel {
             warn!(%key, source = %source, "Received CANCEL for non-existent transaction");
             // Respond 481 Transaction Does Not Exist
             // CANCEL shares same branch as INVITE, but TU handles response
             let response = ResponseBuilder::new(StatusCode::TransactionDoesNotExist)
                .copy_essential_headers(&request) // Add helper or implement here
                .build(); // Assuming build is infallible or handled
             if let Err(e) = transport.send_message(Message::Response(response), source).await {
                error!(error = %e, "Failed to send 481 for CANCEL");
             }
             // Optionally notify TU about stray cancel
             Self::broadcast_event(
                TransactionEvent::StrayCancel { request, source }, // Define this event type
                events_tx,
                event_subscribers
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
                events_tx.clone(), // Pass sender for internal events/responses
            ).map(|tx| Box::new(tx) as BoxedServerTransaction)
        } else {
            ServerNonInviteTransaction::new(
                key.clone(),
                request.clone(),
                source,
                transport.clone(),
                events_tx.clone(),
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
                    events_tx,
                    event_subscribers
                ).await;

                Ok(())
            }
            Err(e) => {
                error!(error = %e, "Failed to create server transaction");
                 Self::broadcast_event(
                    TransactionEvent::Error { error: e.to_string(), transaction_id: Some(key) },
                    events_tx,
                    event_subscribers
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
        mut request: Request, // Take ownership to potentially modify (e.g., add Via)
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        debug!(method = %request.method(), %destination, "Creating client transaction");

        // Ensure top Via header has a branch parameter (essential for transaction matching)
        // Generate branch if not present or if we should always overwrite (depends on strategy)
        let branch = match request.first_via().and_then(|v| v.branch()) {
             Some(b) if !b.starts_with(RFC3261_BRANCH_MAGIC_COOKIE) => {
                 warn!("Existing Via branch does not start with magic cookie, overwriting.");
                 utils::generate_branch()
             }
             Some(b) => b.to_string(), // Use existing valid branch
             None => utils::generate_branch(), // Generate new branch
        };

        // Add/Update Via header
        let via = ViaBuilder::new("SIP", "2.0", "UDP") // TODO: Get transport dynamically
             .host(self.transport.local_addr().await?.ip().to_string()) // Use transport's local IP
             .port(self.transport.local_addr().await?.port())
             .branch(&branch)
             .build()?; // Assuming ViaBuilder exists and returns Result<Via>

        // Prepend the Via header (most recent goes first)
        request.headers_mut().prepend(via);


        // Create the transaction key *after* Via is finalized
         let key = TransactionKey::from_message(&Message::Request(request.clone()))?; // Clone request as key needs it

        let tx_result = if request.method() == Method::Invite {
             debug!(%key, "Creating INVITE client transaction");
             ClientInviteTransaction::new(
                 key.clone(),
                 request, // Pass owned request
                 destination,
                 self.transport.clone(),
                 events_tx.clone(),
             ).map(|tx| Box::new(tx) as BoxedClientTransaction)
         } else {
             debug!(%key, "Creating non-INVITE client transaction");
             ClientNonInviteTransaction::new(
                 key.clone(),
                 request,
                 destination,
                 self.transport.clone(),
                 events_tx.clone(),
             ).map(|tx| Box::new(tx) as BoxedClientTransaction)
         };

        match tx_result {
            Ok(tx) => {
                let transaction_id = tx.id().clone(); // Use the key as ID
                debug!("Created client transaction with ID: {}", transaction_id);

                // Store the transaction and its destination
                 let mut client_txs = self.client_transactions.lock().await;
                 client_txs.insert(transaction_id.clone(), tx); // tx is moved here

                 let mut destinations = self.transaction_destinations.lock().await;
                 destinations.insert(transaction_id.clone(), destination);
                 debug!(%destination, %transaction_id, "Stored destination for transaction");

                // Notify TU (optional, depends if TU needs to know before sending)
                 // self.broadcast_event(
                 //    TransactionEvent::ClientTransactionCreated { transaction_id: transaction_id.clone() }, // Define event
                 //    &self.events_tx,
                 //    &self.event_subscribers
                 // ).await;

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

    /// Send an ACK for a 2xx response (outside of transaction state machine)
    /// This is typically called by the TU (e.g., Session layer) upon receiving a 2xx INVITE response.
    pub async fn send_2xx_ack(
        &self,
        final_response: &Response, // The final (e.g., 200 OK) response received
        // transaction_id: &TransactionKey, // May not be needed if info is in response
        // original_request_uri: &Uri, // Needed for ACK Request-URI
    ) -> Result<()> {

         // ACK Request-URI should be the same as the INVITE Request-URI
         // The final response doesn't reliably contain this. The TU needs to provide it.
         // Let's assume the TU has the original request details or the required URI.
         // For now, we might have to extract from To/Contact if desperate, but it's not ideal.

         // Placeholder: Try to get URI from Contact, fallback to To
         let request_uri = match final_response.typed_header::<Contact>().and_then(|c| c.addresses().first()) {
             Some(contact_addr) => contact_addr.uri().clone(),
             None => match final_response.typed_header::<To>() { // Fallback, less reliable
                 Some(to_hdr) => to_hdr.address().uri().clone(),
                 None => return Err(Error::Other("Cannot determine Request-URI for ACK from response".into())),
             }
         };


         let mut ack_builder = RequestBuilder::new(Method::Ack, &request_uri.to_string())?; // Use RequestBuilder


         // Essential Headers for ACK (RFC 3261 Section 17.1.1.3):
         // Request-URI: Set above.
         // Via: Must equal the Via from the INVITE request, including branch.
         //      The TU should have the original INVITE request or its Via.
         //      We cannot reliably get this from the *response*.
         //      Placeholder: Copy Via from *response* (incorrect by RFC, but might work sometimes)
         if let Some(via) = final_response.first_via() {
             ack_builder = ack_builder.with_header(via.clone());
         } else {
             return Err(Error::Other("Cannot determine Via header for ACK from response".into()));
         }

         // Route: Must equal Route header fields from INVITE request. TU needs original request.
         // Placeholder: Omit Route for now.

         // From: Must equal From header field from INVITE request (incl. tag). Copy from response.
         if let Some(from) = final_response.typed_header::<From>() {
            ack_builder = ack_builder.with_header(from.clone());
         } else {
             return Err(Error::Other("Missing From header in response for ACK".into()));
         }

         // To: Must equal To header field from INVITE request (usually *without* tag initially).
         //    The To header in the 2xx response *will* have a tag. Copy from response.
         if let Some(to) = final_response.typed_header::<To>() {
             ack_builder = ack_builder.with_header(to.clone());
         } else {
             return Err(Error::Other("Missing To header in response for ACK".into()));
         }

         // Call-ID: Must equal Call-ID from INVITE. Copy from response.
         if let Some(call_id) = final_response.call_id() {
             ack_builder = ack_builder.with_header(call_id.clone());
         } else {
             return Err(Error::Other("Missing Call-ID header in response for ACK".into()));
         }

         // CSeq: Must have the CSeq number from the INVITE, but Method is ACK. Copy number from response CSeq.
         if let Some(cseq) = final_response.cseq() {
             ack_builder = ack_builder.with_header(CSeq::new(cseq.sequence(), Method::Ack));
         } else {
             return Err(Error::Other("Missing CSeq header in response for ACK".into()));
         }

         // Max-Forwards: Recommended 70.
         ack_builder = ack_builder.with_header(MaxForwards::new(70));

         // Content-Length: 0 for ACK.
         ack_builder = ack_builder.with_header(ContentLength::new(0));

         // Authentication headers if needed (Proxy-Authorization, Authorization) - TU responsibility.

         let ack_request = ack_builder.build(); // Build the ACK request


        // Determine Destination for ACK (RFC 3261 Section 17.1.1.3):
        // - If INVITE had Route headers, use the first URI in Route set. (TU needs original request)
        // - Else, use the Request-URI of the INVITE. (TU needs original request URI)
        // Placeholder: Use destination calculation logic similar to old code (Contact -> Via -> Registry)
        // This logic is complex and might be better handled by the TU which has more context.

         let destination = Self::determine_ack_destination(final_response).await
             .ok_or_else(|| Error::Other("Could not determine destination for ACK".into()))?;


         info!(%destination, "Sending ACK for 2xx response");

        // Send the ACK directly via transport (it does not use a transaction)
         self.transport.send_message(
             Message::Request(ack_request),
             destination
         ).await.map_err(|e| Error::TransportError(e.to_string())) // Use specific error type
    }

     // Helper to determine ACK destination based on response (best effort)
     async fn determine_ack_destination(response: &Response) -> Option<SocketAddr> {
         // 1. Try Contact header URI
         if let Some(contact) = response.typed_header::<Contact>() {
             if let Some(addr) = contact.addresses().first() {
                  if let Some(dest) = Self::resolve_uri_to_socketaddr(addr.uri()).await {
                      debug!("ACK destination from Contact: {}", dest);
                      return Some(dest);
                  }
             }
         }

         // 2. Try Via header (received/rport if present, otherwise sent-by)
         if let Some(via) = response.first_via() {
              // Prefer received/rport if available
              if let (Some(received_ip_str), Some(rport)) = (via.received(), via.rport()) {
                  if let Ok(ip) = received_ip_str.parse() {
                      let dest = SocketAddr::new(ip, rport);
                      debug!("ACK destination from Via (received/rport): {}", dest);
                      return Some(dest);
                  }
              }
              // Fallback to Via sent-by host/port
              let host = via.sent_by_host();
              let port = via.sent_by_port().unwrap_or(5060); // Default SIP port
              if let Some(dest) = Self::resolve_host_str_to_socketaddr(host, port).await {
                  debug!("ACK destination from Via (sent-by): {}", dest);
                  return Some(dest);
              }
         }

         warn!("Could not determine reliable ACK destination from response headers.");
         None
     }

     // Helper to resolve URI host to SocketAddr (handles IP address and domain)
     async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr> {
         let port = uri.port.unwrap_or(5060); // Default SIP port
         Self::resolve_host_to_socketaddr(&uri.host, port).await
     }

     // Helper to resolve Host enum to SocketAddr
      async fn resolve_host_to_socketaddr(host: &Host, port: u16) -> Option<SocketAddr> {
          match host {
              Host::Address(ip) => Some(SocketAddr::new(*ip, port)),
              Host::Domain(domain) => {
                  // Try direct parse first (might be an IP address string)
                  if let Ok(ip) = domain.parse::<std::net::IpAddr>() {
                      return Some(SocketAddr::new(ip, port));
                  }
                  // Perform DNS lookup (async)
                  match tokio::net::lookup_host(format!("{}:{}", domain, port)).await {
                      Ok(mut addrs) => addrs.next(), // Take the first resolved address
                      Err(e) => {
                          error!(error = %e, domain = %domain, "DNS lookup failed for ACK destination");
                          None
                      }
                  }
              }
          }
      }
     // Helper to resolve Host string to SocketAddr
     async fn resolve_host_str_to_socketaddr(host_str: &str, port: u16) -> Option<SocketAddr> {
         // Try direct parse first
         if let Ok(ip) = host_str.parse::<std::net::IpAddr>() {
             return Some(SocketAddr::new(ip, port));
         }
         // Perform DNS lookup
         match tokio::net::lookup_host(format!("{}:{}", host_str, port)).await {
             Ok(mut addrs) => addrs.next(),
             Err(e) => {
                 error!(error = %e, host = %host_str, "DNS lookup failed");
                 None
             }
         }
     }


}

// Add ResponseBuilder helper trait or standalone function
trait ResponseBuilderExt {
    fn copy_essential_headers(self, request: &Request) -> Self;
}

impl ResponseBuilderExt for ResponseBuilder {
     fn copy_essential_headers(mut self, request: &Request) -> Self {
        // Copy Via (top-most only for responses)
        if let Some(via) = request.first_via() {
             self = self.with_header(via.clone());
         }
         // Copy To (response To usually needs a tag added by TU/Session layer)
         if let Some(to) = request.typed_header::<To>() {
             self = self.with_header(to.clone()); // Tag added later
         }
         // Copy From (response From must match request From, including tag)
         if let Some(from) = request.typed_header::<From>() {
             self = self.with_header(from.clone());
         }
         // Copy Call-ID
         if let Some(call_id) = request.call_id() {
             self = self.with_header(call_id.clone());
         }
         // Copy CSeq
         if let Some(cseq) = request.cseq() {
             self = self.with_header(cseq.clone());
         }
         // Add Content-Length: 0 by default for responses without bodies
         self = self.with_header(ContentLength::new(0));
         self
     }
}


impl fmt::Debug for TransactionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Avoid trying to print the Mutex contents directly, list counts instead
        f.debug_struct("TransactionManager")
            .field("transport", &"Arc<dyn Transport>") // Don't print transport details
            // .field("client_transactions_count", &self.client_transactions.lock().unwrap().len()) // Careful with blocking in fmt
            // .field("server_transactions_count", &self.server_transactions.lock().unwrap().len())
            // .field("destinations_count", &self.transaction_destinations.lock().unwrap().len())
            .field("events_tx", &self.events_tx)
            // .field("event_subscribers_count", &self.event_subscribers.lock().unwrap().len())
            .field("transport_rx", &"Arc<Mutex<Receiver>>") // Don't print receiver details
            .field("running", &self.running) // Arc<Mutex<bool>> can be debugged
            .finish()
    }
}

// Define RFC3261 Branch magic cookie
const RFC3261_BRANCH_MAGIC_COOKIE: &str = "z9hG4bK";
