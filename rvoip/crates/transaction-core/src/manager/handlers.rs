/// # SIP Transaction Message Handlers
///
/// This module implements handlers for processing incoming and outgoing SIP messages through
/// the transaction layer as defined in RFC 3261 Section 17. These handlers are responsible for:
///
/// 1. **Matching** - Matching incoming messages to existing transactions
/// 2. **Routing** - Routing messages to appropriate transactions
/// 3. **State Transitions** - Triggering state transitions in transaction state machines
/// 4. **Special Method Handling** - Processing special cases like ACK, CANCEL, and stray messages
/// 5. **Response Generation** - Automatically generating specific responses (e.g., 200 OK for CANCEL)
///
/// The handlers implement the core logic required for the transaction layer to fulfill its role
/// as the reliability layer between the Transport layer and the Transaction User (TU).
///
/// ## RFC 3261 Specification Coverage
///
/// These handlers implement the behavior required by:
/// - Section 17.1.3: Matching responses to client transactions
/// - Section 17.2.3: Matching requests to server transactions
/// - Section 8.2.6: Generating automatic responses
/// - Section 9.2: CANCEL handling
/// - Section 17.1.1.3: ACK handling

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::str::FromStr;

use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::json::ext::SipMessageJson;
use rvoip_sip_transport::{Transport, TransportEvent};

use crate::error::{self, Error, Result};
use crate::transaction::{
    Transaction, TransactionAsync, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand,
};
use crate::client::{ClientTransaction, TransactionExt as ClientTransactionExt, ClientInviteTransaction, ClientNonInviteTransaction};
use crate::server::{ServerTransaction, TransactionExt as ServerTransactionExt, ServerInviteTransaction, ServerNonInviteTransaction};
use crate::utils::{self, transaction_key_from_message, create_ack_from_invite};

use super::TransactionManager;
use super::types::*;

/// Handle transport message events and route them to appropriate transactions.
///
/// This is the main entry point for all incoming SIP messages from the transport
/// layer. It implements the message matching rules specified in RFC 3261 sections
/// 17.1.3 (client transactions) and 17.2.3 (server transactions).
///
/// The function:
/// 1. Identifies the transaction that should handle the message
/// 2. Routes requests/responses to appropriate transactions
/// 3. Handles special cases (ACK, CANCEL)
/// 4. Reports "stray" messages that don't match any transaction
///
/// # Arguments
/// * `event` - The transport event containing the message and addressing information
/// * `transport` - The transport layer for sending responses
/// * `client_transactions` - Map of active client transactions
/// * `server_transactions` - Map of active server transactions
/// * `events_tx` - Channel for broadcasting transaction events
/// * `event_subscribers` - Additional event subscribers
/// * `manager` - Reference to the TransactionManager
///
/// # Returns
/// * `Result<()>` - Success or error depending on message processing outcome
pub async fn handle_transport_message(
    event: TransportEvent,
    transport: &Arc<dyn Transport>,
    client_transactions: &Arc<Mutex<HashMap<TransactionKey, Box<dyn ClientTransaction + Send>>>>,
    server_transactions: &Arc<Mutex<HashMap<TransactionKey, Arc<dyn ServerTransaction>>>>,
    events_tx: &mpsc::Sender<TransactionEvent>,
    event_subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
    manager: &TransactionManager,
) -> Result<()> {
    match event {
        TransportEvent::MessageReceived { message, source, destination } => {
            match message {
                Message::Request(request) => {
                    // First, determine the transaction ID/key
                    let tx_id = match transaction_key_from_message(&Message::Request(request.clone())) {
                        Some(key) => key,
                        None => {
                            return Err(Error::Other("Could not determine transaction ID from request".into()));
                        }
                    };
                    
                    // Handle ACK specially
                    if request.method() == Method::Ack {
                        // Clone the request before we start working with it
                        let ack_request = request.clone();
                        
                        // Get a lock on server transactions
                        let server_txs = server_transactions.lock().await;
                        
                        // Check if we have a direct matching transaction
                        let tx_opt = if server_txs.contains_key(&tx_id) {
                            Some(server_txs[&tx_id].clone())
                        } else {
                            // If not directly matched, try to find a matching INVITE transaction
                            // based on dialog identifiers (Call-ID, From tag, To tag)
                            let matching_tx = server_txs.iter()
                                .filter(|(key, tx)| {
                                    // Only look at INVITE server transactions
                                    key.method() == &Method::Invite && key.is_server()
                                })
                                .find_map(|(key, tx)| {
                                    // Try to match based on dialog identifiers
                                    if let (Some(req_call_id), Some(ack_call_id)) = (tx.original_request_call_id(), ack_request.call_id()) {
                                        if req_call_id == ack_call_id.value() {
                                            // Found potential match by Call-ID, now check From/To tags
                                            if let (Some(req_from), Some(ack_from)) = (tx.original_request_from_tag(), ack_request.from_tag()) {
                                                if req_from == ack_from {
                                                    // If To tag exists in both, it should match
                                                    let to_matches = match (tx.original_request_to_tag(), ack_request.to_tag()) {
                                                        (Some(req_to), Some(ack_to)) => req_to == ack_to,
                                                        // In early dialogs, original request might not have To tag
                                                        _ => true
                                                    };
                                                    
                                                    if to_matches {
                                                        debug!(call_id=%req_call_id, "Found matching INVITE server transaction for ACK by dialog identifiers");
                                                        return Some(tx.clone());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    None
                                });
                            
                            matching_tx
                        };
                        
                        // If we found a transaction, process the ACK
                        if let Some(tx) = tx_opt {
                            let tx_id = tx.id().clone();
                            let tx_kind = tx.kind();
                            
                            if tx_kind == TransactionKind::InviteServer {
                                debug!(%tx_id, "Processing ACK for server INVITE transaction");
                                
                                // Clone what we need so we can release the lock
                                let tx_clone = tx.clone();
                                
                                // Drop the lock before async operations
                                drop(server_txs);
                                
                                // Use a timeout to avoid blocking indefinitely if the transaction is shutting down
                                match tokio::time::timeout(
                                    std::time::Duration::from_millis(500), 
                                    tx_clone.process_request(ack_request.clone())
                                ).await {
                                    Ok(result) => {
                                        // Process the result
                                        match result {
                                            Ok(_) => {
                                                // Successfully processed ACK
                                                
                                                // Broadcast the event
                                                TransactionManager::broadcast_event(
                                                    TransactionEvent::AckReceived {
                                                        transaction_id: tx_id.clone(),
                                                        request: ack_request,
                                                    },
                                                    events_tx,
                                                    event_subscribers,
                                                    None,
                                                    None,
                                                ).await;
                                                
                                                return Ok(());
                                            },
                                            Err(e) => {
                                                // Transaction error - likely channel closed
                                                warn!(%tx_id, error=%e, "Failed to process ACK request, treating as stray ACK");
                                                // Fall through to stray ACK handling
                                            }
                                        }
                                    },
                                    Err(_) => {
                                        // Timeout waiting for transaction to process ACK
                                        warn!(%tx_id, "Timeout processing ACK request, treating as stray ACK");
                                        // Fall through to stray ACK handling
                                    }
                                }
                            } else {
                                // Transaction is not an INVITE server transaction
                                drop(server_txs);
                                // Fall through to stray ACK handling
                            }
                        } else {
                            // No matching transaction found
                            drop(server_txs);
                            // Fall through to stray ACK handling
                        }
                        
                        // Handle as stray ACK if we reached this point
                        debug!("Received ACK that doesn't match any server transaction");
                        TransactionManager::broadcast_event(
                            TransactionEvent::StrayAck {
                                request: ack_request,
                                source,
                            },
                            events_tx,
                            event_subscribers,
                            None,
                            None,
                        ).await;
                        
                        return Ok(());
                    }
                    
                    // Handle CANCEL specially
                    if request.method() == Method::Cancel {
                        // Extract the branch parameter from the CANCEL request
                        let cancel_branch = match request.first_via() {
                            Some(via) => match via.branch() {
                                Some(branch) => branch.to_string(),
                                None => {
                                    debug!("CANCEL request has no branch parameter, can't find matching INVITE");
                                    // Fall through to stray CANCEL handling
                                    handle_stray_cancel(request.clone(), source, transport).await?;
                                    
                                    // Broadcast stray CANCEL event
                                    TransactionManager::broadcast_event(
                                        TransactionEvent::StrayCancel {
                                            request,
                                            source,
                                        },
                                        events_tx,
                                        event_subscribers,
                                        None,
                                        None,
                                    ).await;
                                    return Ok(());
                                }
                            },
                            None => {
                                debug!("CANCEL request has no Via header, can't find matching INVITE");
                                // Fall through to stray CANCEL handling
                                handle_stray_cancel(request.clone(), source, transport).await?;
                                
                                // Broadcast stray CANCEL event
                                TransactionManager::broadcast_event(
                                    TransactionEvent::StrayCancel {
                                        request,
                                        source,
                                    },
                                    events_tx,
                                    event_subscribers,
                                    None,
                                    None,
                                ).await;
                                return Ok(());
                            }
                        };
                        
                        // Create a modified key for the INVITE transaction with the same branch
                        let invite_tx_id = TransactionKey::new(cancel_branch, Method::Invite, true);
                        
                        debug!("Looking for INVITE transaction with key: {}", invite_tx_id);
                        
                        // Check if we have a matching INVITE transaction with the same branch
                        let tx_clone_opt = {
                            let server_txs = server_transactions.lock().await;
                            if server_txs.contains_key(&invite_tx_id) {
                                let tx = server_txs.get(&invite_tx_id).unwrap();
                                if tx.kind() == TransactionKind::InviteServer {
                                    // Clone the transaction while we have the lock
                                    Some((tx.clone(), invite_tx_id.clone()))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                            // Lock is released at the end of this block
                        };
                        
                        if let Some((tx, tx_id_clone)) = tx_clone_opt {
                            // Now proceed with the transaction outside the lock
                            let request_clone = request.clone();
                            
                            debug!(%tx_id_clone, "Processing CANCEL for server INVITE transaction");
                            
                            // Broadcast event
                            TransactionManager::broadcast_event(
                                TransactionEvent::CancelReceived {
                                    transaction_id: tx_id_clone.clone(),
                                    cancel_request: request_clone.clone(),
                                },
                                events_tx,
                                event_subscribers,
                                None,
                                None,
                            ).await;
                            
                            // Send OK response to CANCEL
                            let mut builder = ResponseBuilder::new(StatusCode::Ok, None);
                            
                            // Add necessary headers
                            if let Some(to) = request_clone.to() {
                                builder = builder.header(TypedHeader::To(to.clone()));
                            }
                            
                            if let Some(from) = request_clone.from() {
                                builder = builder.header(TypedHeader::From(from.clone()));
                            }
                            
                            if let Some(call_id) = request_clone.call_id() {
                                builder = builder.header(TypedHeader::CallId(call_id.clone()));
                            }
                            
                            if let Some(cseq) = request_clone.cseq() {
                                builder = builder.header(TypedHeader::CSeq(cseq.clone()));
                            }
                            
                            if let Some(via) = request_clone.header(&HeaderName::Via) {
                                builder = builder.header(via.clone());
                            }
                            
                            // Build and send response to CANCEL
                            let cancel_response = builder.build();
                            if let Err(e) = transport
                                .send_message(Message::Response(cancel_response), source)
                                .await {
                                return Err(Error::transport_error(e, "Failed to send 200 OK response to CANCEL"));
                            }
                            
                            // Now send 487 Request Terminated for the original INVITE
                            debug!(%tx_id_clone, "Sending 487 Request Terminated for the original INVITE");
                            
                            let mut builder = ResponseBuilder::new(StatusCode::RequestTerminated, None);
                            
                            if let Some(invite_request) = tx.original_request().await {
                                // Add necessary headers from the INVITE request
                                if let Some(to) = invite_request.to() {
                                    builder = builder.header(TypedHeader::To(to.clone()));
                                }
                                
                                if let Some(from) = invite_request.from() {
                                    builder = builder.header(TypedHeader::From(from.clone()));
                                }
                                
                                if let Some(call_id) = invite_request.call_id() {
                                    builder = builder.header(TypedHeader::CallId(call_id.clone()));
                                }
                                
                                if let Some(cseq) = invite_request.cseq() {
                                    builder = builder.header(TypedHeader::CSeq(cseq.clone()));
                                }
                                
                                if let Some(via) = invite_request.header(&HeaderName::Via) {
                                    builder = builder.header(via.clone());
                                }
                                
                                // Build the 487 response
                                let invite_response = builder.build();
                                
                                // Instead of sending directly through the transport, 
                                // send through the transaction's send_response method
                                // This ensures proper state transition and processing
                                if let Err(e) = tx.send_response(invite_response).await {
                                    warn!(%tx_id_clone, error=%e, "Failed to send 487 Request Terminated through transaction");
                                    return Err(Error::Other(format!("Failed to send 487 Request Terminated: {}", e)));
                                }
                            }
                            
                            return Ok(());
                        }
                        
                        // If no matching transaction was found, handle as stray CANCEL
                        debug!("Received CANCEL that doesn't match any INVITE server transaction");
                        handle_stray_cancel(request.clone(), source, transport).await?;
                        
                        // Broadcast stray CANCEL event
                        TransactionManager::broadcast_event(
                            TransactionEvent::StrayCancel {
                                request,
                                source,
                            },
                            events_tx,
                            event_subscribers,
                            None,
                            None,
                        ).await;
                        return Ok(());
                    }
                    
                    // Handle regular request retransmission and new requests
                    let mut server_txs = server_transactions.lock().await;
                    
                    // Check if we have a matching transaction
                    if server_txs.contains_key(&tx_id) {
                        debug!(%tx_id, "Processing retransmission of existing request");
                        
                        // Process the request while still holding the lock
                        // The implementation of process_request will handle async operations properly
                        let result = server_txs[&tx_id].process_request(request.clone()).await;
                        
                        // Now we can drop the lock
                        drop(server_txs);
                        
                        // Check for errors
                        result?;
                        
                        return Ok(());
                    }
                    
                    // Drop the lock
                    drop(server_txs);
                    
                    // If we get here, this is a new request
                    debug!(%tx_id, method = ?request.method(), "Received new request, delegate to proper handler");
                    
                    // Delegate to the actual request handler which will create appropriate transactions
                    // and generate the correct InviteRequest or NonInviteRequest events
                    if let Err(e) = manager.handle_request(request, source).await {
                        warn!(error=%e, "Failed to handle new request");
                    }
                    
                    return Ok(());
                },
                Message::Response(response) => {
                    // Try to match the response to a client transaction by deriving its ID
                    let tx_id = match transaction_key_from_message(&Message::Response(response.clone())) {
                        Some(key) => key,
                        None => {
                            return Err(Error::Other("Could not determine transaction ID from response".into()));
                        }
                    };
                    
                    // Look up the client transaction using the same pattern
                    let mut client_txs = client_transactions.lock().await;
                    
                    // Check if we have a matching transaction
                    if client_txs.contains_key(&tx_id) {
                        let tx_kind = client_txs[&tx_id].kind();
                        let remote_addr = client_txs[&tx_id].remote_addr();
                        
                        debug!(%tx_id, status = ?response.status(), "Routing response to client transaction");
                        
                        // Process the response while still holding the lock
                        let result = client_txs[&tx_id].process_response(response.clone()).await;
                        
                        // Now we can drop the lock
                        drop(client_txs);
                        
                        // Check for errors
                        result?;
                        
                        // Automatic ACK for non-2xx responses to INVITE
                        if !response.status().is_success() && tx_kind == TransactionKind::InviteClient {
                            debug!(%tx_id, status=%response.status(), "Sending ACK automatically for non-2xx response");
                            
                            // Create a dummy request for ACK creation
                            let dummy_uri = if let Some(to) = response.to() {
                                to.address().uri.clone()
                            } else {
                                Uri::sip("invalid")
                            };
                            
                            let dummy_request = Request::new(Method::Invite, dummy_uri);
                            
                            match create_ack_from_invite(&dummy_request, &response) {
                                Ok(ack_request) => {
                                    // Send the ACK
                                    if let Err(e) = transport
                                        .send_message(Message::Request(ack_request), remote_addr)
                                        .await {
                                        return Err(Error::transport_error(e, "Failed to send ACK for non-2xx response"));
                                    }
                                },
                                Err(e) => {
                                    warn!(%tx_id, error=%e, "Failed to create ACK request");
                                }
                            }
                        }
                        
                        return Ok(());
                    }
                    
                    // Drop the lock
                    drop(client_txs);
                    
                    // If we get here, this is a stray response
                    debug!(status=%response.status(), "Received stray response that doesn't match any client transaction");
                    
                    // Broadcast stray response event
                    TransactionManager::broadcast_event(
                        TransactionEvent::StrayResponse {
                            response,
                            source,
                        },
                        events_tx,
                        event_subscribers,
                        None,
                        None,
                    ).await;
                    
                    return Ok(());
                }
            }
        },
        TransportEvent::Error { error } => {
            warn!("Transport error: {}", error);
            // TODO: Determine if any transactions were affected by this error
            // and propagate the error to them
        },
        _ => {
            // Ignore other transport events for now
        }
    }
    
    Ok(())
}

/// Determine ACK destination for 2xx responses according to RFC 3261 Section 13.2.2.4.
///
/// For 2xx responses to INVITE, ACK requests are sent directly from the TU to the peer.
/// This function implements the algorithm to determine where to send the ACK based on:
/// 1. The Contact header if present
/// 2. Fallback to Via header received/rport parameters or sent-by value
///
/// # Arguments
/// * `response` - The 2xx response to ACK
///
/// # Returns
/// * `Option<SocketAddr>` - The destination socket address if it can be determined
pub async fn determine_ack_destination(response: &Response) -> Option<SocketAddr> {
    if let Some(contact_header) = response.header(&HeaderName::Contact) {
        if let TypedHeader::Contact(contact) = contact_header {
            if let Some(addr) = contact.addresses().next() {
                if let Some(dest) = resolve_uri_to_socketaddr(&addr.uri).await {
                    return Some(dest);
                }
            }
        }
    }
    
    // Try via received/rport
    if let Some(via) = response.first_via() {
        if let (Some(received_ip_str), Some(port)) = (via.received().map(|ip| ip.to_string()), via.rport().flatten()) {
            if let Ok(ip) = IpAddr::from_str(&received_ip_str) {
                let dest = SocketAddr::new(ip, port);
                return Some(dest);
            } else {
                warn!(ip=%received_ip_str, "Failed to parse received IP in Via");
            }
        }
        
        // Fallback to Via host/port
        // For the sent_by, use ViaHeader struct fields
        if let Some(via_header) = via.headers().first() {
            let host = &via_header.sent_by_host;
            let port = via_header.sent_by_port.unwrap_or(5060);
            
            if let Some(dest) = resolve_host_to_socketaddr(host, port).await {
                return Some(dest);
            }
        }
    }
    None
}

/// Helper to resolve URI host to SocketAddr for ACK destinations.
///
/// This implements the address resolution for SIP URIs according to
/// RFC 3263 procedures.
///
/// # Arguments
/// * `uri` - SIP URI to resolve
///
/// # Returns
/// * `Option<SocketAddr>` - Resolved socket address if successful
async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr> {
    let port = uri.port.unwrap_or(5060);
    resolve_host_to_socketaddr(&uri.host, port).await
}

/// Helper to resolve Host enum to SocketAddr for network addressing.
///
/// SIP specification allows both IP addresses and domain names as hosts.
/// This function resolves them to socket addresses for actual transmission.
///
/// # Arguments
/// * `host` - SIP host to resolve (IP or domain)
/// * `port` - Port number to use
///
/// # Returns
/// * `Option<SocketAddr>` - Resolved socket address if successful
async fn resolve_host_to_socketaddr(host: &rvoip_sip_core::Host, port: u16) -> Option<SocketAddr> {
    match host {
        rvoip_sip_core::Host::Address(ip) => Some(SocketAddr::new(*ip, port)),
        rvoip_sip_core::Host::Domain(domain) => {
            if let Ok(ip) = IpAddr::from_str(domain) {
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

/// Create a Via header with a branch parameter for a local address
pub fn create_via_header(local_addr: &SocketAddr, branch: &str) -> Result<TypedHeader> {
    use rvoip_sip_core::types::via::Via;
    use rvoip_sip_core::types::Param;
    
    // Create a new Via header with the specified branch parameter
    let mut via_params = vec![Param::branch(branch.to_string())];
    
    // Add other params like rport if needed
    // Example: via_params.push(Param::other("rport".to_string(), None));
    
    // Create the Via header
    let local_host = local_addr.ip().to_string();
    let local_port = local_addr.port();
    
    let via = Via::new(
        "SIP", "2.0", "UDP",
        &local_host, Some(local_port), via_params
    ).map_err(Error::SipCoreError)?;
    
    Ok(TypedHeader::Via(via))
}

impl TransactionManager {
    /// Handle an incoming SIP message from the transport layer.
    ///
    /// This is the entry point for incoming messages from the transport layer to
    /// the TransactionManager. It delegates to more specific handlers based on
    /// message type.
    ///
    /// # Arguments
    /// * `event` - Transport event containing the message and addressing information
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on message processing outcome
    pub(crate) async fn handle_transport_event(&self, event: TransportEvent) -> Result<()> {
        match event {
            TransportEvent::MessageReceived { message, source, destination } => {
                debug!("Received message from {}", source);
                self.handle_message(message, source, destination).await
            },
            _ => {
                // We don't care about other transport events for now
                Ok(())
            }
        }
    }
    
    /// Handle a SIP message, routing it to appropriate transaction or creating a new one.
    ///
    /// This method dispatches incoming messages to either request or response
    /// handlers, which implement the core transaction layer logic.
    ///
    /// # Arguments
    /// * `message` - The SIP message (request or response)
    /// * `source` - The source address of the message
    /// * `destination` - The local address that received the message
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on message processing outcome
    async fn handle_message(
        &self,
        message: Message,
        source: SocketAddr,
        destination: SocketAddr,
    ) -> Result<()> {
        match message {
            Message::Request(request) => {
                // Special handling for ACK to 2xx responses
                if request.method() == Method::Ack {
                    // ACK requests matching a 2xx response are end-to-end and don't have a transaction
                    return self.handle_ack_request(request, source).await;
                }
                
                self.handle_request(request, source).await
            },
            Message::Response(response) => {
                self.handle_response(response, source).await
            }
        }
    }
    
    /// Handle an incoming SIP request according to RFC 3261 transaction rules.
    ///
    /// This method:
    /// 1. Attempts to match the request to an existing server transaction
    /// 2. Creates a new server transaction if no match is found
    /// 3. Notifies the TU about the request based on its method
    ///
    /// # Arguments
    /// * `request` - The incoming SIP request
    /// * `source` - The source address of the request
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on request processing outcome
    async fn handle_request(&self, request: Request, source: SocketAddr) -> Result<()> {
        // Try to find a matching transaction
        if let Some(key) = crate::utils::transaction_key_from_message(&Message::Request(request.clone())) {
            // Check for existing server transaction
            let server_txs = self.server_transactions.lock().await;
            if let Some(transaction) = server_txs.get(&key) {
                let tx = transaction.clone();
                drop(server_txs);
                
                // Process the request in the existing transaction
                return tx.process_request(request).await;
            }
            drop(server_txs);
        }
        
        // No existing transaction found, create a new one
        let transaction = self.create_server_transaction(request.clone(), source).await?;
        
        // Notify the transaction user about the new transaction
        match transaction.kind() {
            TransactionKind::InviteServer => {
                self.events_tx.send(crate::TransactionEvent::InviteRequest {
                    transaction_id: transaction.id().clone(),
                    request,
                    source,
                }).await.ok();
            },
            TransactionKind::NonInviteServer => {
                // For non-INVITE requests, notify based on the method
                match request.method() {
                    Method::Cancel => {
                        // CANCEL events are handled in create_server_transaction
                        // to link them with the target INVITE transaction
                    },
                    _ => {
                        self.events_tx.send(crate::TransactionEvent::NonInviteRequest {
                            transaction_id: transaction.id().clone(),
                            request,
                            source,
                        }).await.ok();
                    }
                }
            },
            // Client transaction kinds shouldn't occur here, but handle them for completeness
            TransactionKind::InviteClient | TransactionKind::NonInviteClient => {
                warn!("Unexpected client transaction kind in handle_request");
            },
        }
        
        Ok(())
    }
    
    /// Handle an incoming SIP response according to RFC 3261 transaction rules.
    ///
    /// This method:
    /// 1. Attempts to match the response to an existing client transaction
    /// 2. Delivers the response to the matched transaction
    /// 3. Generates a "stray response" event if no match is found
    ///
    /// # Arguments
    /// * `response` - The incoming SIP response
    /// * `source` - The source address of the response
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on response processing
    async fn handle_response(&self, response: Response, source: SocketAddr) -> Result<()> {
        // Try to find a matching client transaction
        if let Some(key) = crate::utils::transaction_key_from_message(&Message::Response(response.clone())) {
            debug!(id=%key, "Found matching transaction for response");
            
            // Check that the key is for a client transaction (not is_server)
            if key.is_server() {
                // This is a response but the transaction key is for a server - this is a mismatch
                return Err(Error::Other(format!(
                    "Received response but matching transaction key {} is for a server transaction", key
                )));
            }

            // First try to send directly to the transaction via events_tx
            let mut client_txs_guard = self.client_transactions.lock().await;
            let mut processed = false;
            
            if client_txs_guard.contains_key(&key) {
                let transaction = client_txs_guard.remove(&key).unwrap();
                
                // Drop the lock so we can do async operations
                drop(client_txs_guard);
                
                // Process the response
                if let Err(e) = transaction.process_response(response.clone()).await {
                    warn!(id=%key, error=%e, "Error processing response");
                } else {
                    processed = true;
                }
                
                // Put the transaction back
                let mut client_txs_guard = self.client_transactions.lock().await;
                client_txs_guard.insert(key.clone(), transaction);
                drop(client_txs_guard);
            } else {
                drop(client_txs_guard);
            }
            
            // If not processed via transaction, still send the event
            if !processed {
                debug!(id=%key, "Response matches key but no active transaction found");
                
                // Deliver to the transaction user anyway
                let status = response.status();
                if key.method() == &Method::Invite && status.is_success() {
                    // Special handling for 2xx responses to INVITE
                    self.events_tx.send(crate::TransactionEvent::SuccessResponse {
                        transaction_id: key,
                        response,
                        need_ack: true,
                        source,
                    }).await.ok();
                } else {
                    // All other responses - classify by status code
                    let status_code = response.status_code();
                    if status_code >= 100 && status_code < 200 {
                        // 1xx provisional response
                        self.events_tx.send(crate::TransactionEvent::ProvisionalResponse {
                            transaction_id: key,
                            response,
                        }).await.ok();
                    } else if status.is_success() && key.method() != &Method::Invite {
                        // 2xx success response for non-INVITE
                        self.events_tx.send(crate::TransactionEvent::SuccessResponse {
                            transaction_id: key,
                            response,
                            need_ack: false,
                            source,
                        }).await.ok();
                    } else {
                        // 3xx, 4xx, 5xx, 6xx failure response
                        self.events_tx.send(crate::TransactionEvent::FailureResponse {
                            transaction_id: key,
                            response,
                        }).await.ok();
                    }
                }
            }
            
            return Ok(());
        }
        
        // No transaction match
        debug!("No matching transaction found for response");
        
        // This could be a response for a transaction that has already terminated
        // or a response forwarded by another SIP entity (for proxy scenarios)
        // In any case, deliver it to the transaction user
        self.events_tx.send(crate::TransactionEvent::StrayResponse {
            response,
            source,
        }).await.ok();
        
        Ok(())
    }
    
    /// Handle an ACK request with special processing required by RFC 3261.
    ///
    /// ACK is a special method in SIP:
    /// - ACK for non-2xx responses is part of the INVITE transaction
    /// - ACK for 2xx responses is a separate end-to-end transaction
    ///
    /// This method handles both cases according to RFC 3261 Section 17.1.1.3.
    ///
    /// # Arguments
    /// * `request` - The ACK request
    /// * `source` - The source address of the request
    ///
    /// # Returns
    /// * `Result<()>` - Success or error depending on ACK processing
    async fn handle_ack_request(&self, request: Request, source: SocketAddr) -> Result<()> {
        // Check if this ACK matches an INVITE server transaction
        if let Some(key) = crate::utils::transaction_key_from_message(&Message::Request(request.clone())) {
            // Create a key for the INVITE transaction this ACK might belong to
            // ACK has its own branch parameter but the To and From tags should match
            // the INVITE transaction
            let invite_key = key.with_method(Method::Invite);
            
            // Check for an INVITE server transaction
            let server_txs = self.server_transactions.lock().await;
            if let Some(transaction) = server_txs.get(&invite_key) {
                if transaction.state() == TransactionState::Confirmed {
                    // This is an ACK for a 2xx response, which is end-to-end
                    // and doesn't have its own transaction
                    self.events_tx.send(crate::TransactionEvent::AckRequest {
                        transaction_id: invite_key,
                        request,
                        source,
                    }).await.ok();
                    
                    return Ok(());
                } else {
                    // This is an ACK for a non-2xx response, process it in the transaction
                    let tx = transaction.clone();
                    drop(server_txs);
                    
                    return tx.process_request(request).await;
                }
            }
            drop(server_txs);
        }
        
        // No matching INVITE transaction found, this is a stray ACK
        debug!("No matching INVITE transaction found for ACK request");
        
        // Notify the transaction user about the stray ACK
        self.events_tx.send(crate::TransactionEvent::StrayAckRequest {
            request,
            source,
        }).await.ok();
        
        Ok(())
    }
}

/// Helper function to handle stray CANCEL requests
async fn handle_stray_cancel(
    request: Request, 
    source: SocketAddr,
    transport: &Arc<dyn Transport>,
) -> Result<()> {
    // Send 481 Transaction Does Not Exist
    let mut builder = ResponseBuilder::new(StatusCode::CallOrTransactionDoesNotExist, None);
    
    // Add necessary headers
    if let Some(to) = request.to() {
        builder = builder.header(TypedHeader::To(to.clone()));
    }
    
    if let Some(from) = request.from() {
        builder = builder.header(TypedHeader::From(from.clone()));
    }
    
    if let Some(call_id) = request.call_id() {
        builder = builder.header(TypedHeader::CallId(call_id.clone()));
    }
    
    if let Some(cseq) = request.cseq() {
        builder = builder.header(TypedHeader::CSeq(cseq.clone()));
    }
    
    if let Some(via) = request.header(&HeaderName::Via) {
        builder = builder.header(via.clone());
    }
    
    // Build the response
    let cancel_response = builder.build();
    
    // Send the response
    if let Err(e) = transport
        .send_message(Message::Response(cancel_response), source)
        .await {
        return Err(Error::transport_error(e, "Failed to send 481 response to stray CANCEL"));
    }
    
    Ok(())
} 