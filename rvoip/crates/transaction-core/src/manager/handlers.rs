use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::str::FromStr;

use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};

use rvoip_sip_core::prelude::*;
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

/// Handle transport message events and route them to appropriate transactions
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
                        let mut server_txs = server_transactions.lock().await;
                        
                        // Check if we have a matching transaction
                        if server_txs.contains_key(&tx_id) {
                            let tx_kind = server_txs[&tx_id].kind();
                            
                            if tx_kind == TransactionKind::InviteServer {
                                debug!(%tx_id, "Processing ACK for server INVITE transaction");
                                
                                // Process the request while still holding the lock
                                // The implementation of process_request will handle async operations properly
                                let result = server_txs[&tx_id].process_request(request.clone()).await;
                                
                                // Now we can drop the lock
                                drop(server_txs);
                                
                                // Check for errors
                                result?;
                                
                                // Broadcast the event after dropping lock
                                TransactionManager::broadcast_event(
                                    TransactionEvent::AckReceived {
                                        transaction_id: tx_id.clone(),
                                        request,
                                    },
                                    events_tx,
                                    event_subscribers,
                                    None,
                                ).await;
                                
                                return Ok(());
                            }
                        }
                        
                        // Release the lock if transaction not found
                        drop(server_txs);
                        
                        // Handle stray ACK
                        debug!("Received ACK that doesn't match any server transaction");
                        TransactionManager::broadcast_event(
                            TransactionEvent::StrayAck {
                                request,
                                source,
                            },
                            events_tx,
                            event_subscribers,
                            None,
                        ).await;
                        
                        return Ok(());
                    }
                    
                    // Handle CANCEL specially
                    if request.method() == Method::Cancel {
                        // Handle CANCEL using the same lock pattern
                        let server_txs = server_transactions.lock().await;
                        if let Some(tx) = server_txs.get(&tx_id) {
                            if tx.kind() == TransactionKind::InviteServer {
                                // Make a clone of what we need
                                let request_clone = request.clone();
                                let tx_id_clone = tx_id.clone();
                                
                                // Release the lock before async operations
                                drop(server_txs);
                                
                                debug!(%tx_id, "Processing CANCEL for server INVITE transaction");
                                
                                // Broadcast event
                                TransactionManager::broadcast_event(
                                    TransactionEvent::CancelReceived {
                                        transaction_id: tx_id_clone,
                                        cancel_request: request_clone.clone(),
                                    },
                                    events_tx,
                                    event_subscribers,
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
                                
                                // Build and send response
                                let cancel_response = builder.build();
                                if let Err(e) = transport
                                    .send_message(Message::Response(cancel_response), source)
                                    .await {
                                    return Err(Error::transport_error(e, "Failed to send 200 OK response to CANCEL"));
                                }
                                
                                return Ok(());
                            }
                        }
                        
                        // Drop the lock if we didn't match
                        drop(server_txs);
                        
                        // Handle stray CANCEL
                        debug!("Received CANCEL that doesn't match any server transaction");
                        
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
                        
                        // Broadcast stray CANCEL event
                        TransactionManager::broadcast_event(
                            TransactionEvent::StrayCancel {
                                request,
                                source,
                            },
                            events_tx,
                            event_subscribers,
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
                    debug!(%tx_id, method = ?request.method(), "Received new request, notify TU");
                    
                    // Notify TU about new request
                    TransactionManager::broadcast_event(
                        TransactionEvent::NewRequest {
                            transaction_id: tx_id,
                            request,
                            source,
                        },
                        events_tx,
                        event_subscribers,
                        None,
                    ).await;
                    
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

/// Determine ACK destination for 2xx responses
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

/// Helper to resolve URI host to SocketAddr
async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr> {
    let port = uri.port.unwrap_or(5060);
    resolve_host_to_socketaddr(&uri.host, port).await
}

/// Helper to resolve Host enum to SocketAddr
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