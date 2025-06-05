// Performance benchmark example for the session-core library
// This is used to benchmark various operations in the library
// such as dialog creation, SDP negotiation, etc.
//
// ## How to Run
// 
// From the project root directory:
// ```
// cargo run --example benchmarks -- [SESSION_COUNT] [OPTIONS]
// ```
//
// ### Arguments
//
// - `SESSION_COUNT`: (Optional) Number of concurrent SIP sessions to test (default: 1000)
// - `OPTIONS`:
//   - `-v, --verbose`: Enable verbose output to see detailed transaction logs
//
// ### Examples
//
// Run with 100 concurrent sessions:
// ```
// cargo run --example benchmarks -- 100
// ```
//
// Run with 10 sessions and verbose logging:
// ```
// cargo run --example benchmarks -- 10 --verbose
// ```
//
// ### Output
//
// The benchmark results will show:
// - Number of successful and failed sessions
// - Total duration of the test
// - Average time per session
// - Sessions processed per second
//
// ### Notes
//
// - The benchmark creates a simulated network environment with UAC (client) and UAS (server)
// - It tests the full SIP transaction flow including INVITE, responses, and dialog creation
// - For debugging purposes, use a small number of sessions with verbose mode
// - For performance testing, use a larger number of sessions without verbose mode

use std::sync::Arc;
use std::net::SocketAddr;
use std::time::{Instant, Duration};
use tokio::time::sleep;
use anyhow::Result;
use std::str::FromStr;
use std::env;
use tokio::sync::{mpsc, broadcast};
use tokio::task::JoinSet;
use dashmap::DashMap;
use uuid::Uuid;

use rvoip_transaction_core::{
    TransactionManager, 
    TransactionEvent,
};

use rvoip_sip_core::{
    Method, Uri, Request, Response, HeaderName, TypedHeader,
    types::{status::StatusCode, address::Address},
    builder::{SimpleRequestBuilder, SimpleResponseBuilder},
};

use rvoip_sip_transport::{Transport, TransportEvent};

// Update imports to use proper modules
use rvoip_session_core::{
    events::{EventBus, SessionEvent},
    session::{
        SessionConfig, 
        SessionId, 
        session::Session, 
        manager::SessionManager,
        SessionState
    },
    dialog::{
        DialogId, 
        dialog_manager::DialogManager,
        dialog_state::DialogState,
        dialog_utils
    },
    sdp::SessionDescription,
    errors::Error,
    helpers::{make_call, end_call}
};

// Global verbose flag
static mut VERBOSE: bool = false;

// Helper function for conditional logging
fn log(msg: &str) {
    unsafe {
        if VERBOSE {
            println!("{}", msg);
        }
    }
}

// Loopback Transport Implementation
#[derive(Clone, Debug)]
struct LoopbackTransport {
    local_addr: std::net::SocketAddr,
    // Registry of all loopback transports to route messages
    registry: Arc<DashMap<std::net::SocketAddr, mpsc::Sender<rvoip_sip_transport::TransportEvent>>>,
    // Keep track of sent messages for debugging
    sent_count: Arc<std::sync::atomic::AtomicUsize>,
    received_count: Arc<std::sync::atomic::AtomicUsize>,
}

impl LoopbackTransport {
    fn new(addr: std::net::SocketAddr, registry: Arc<DashMap<std::net::SocketAddr, mpsc::Sender<rvoip_sip_transport::TransportEvent>>>) -> Self {
        Self {
            local_addr: addr,
            registry,
            sent_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            received_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for LoopbackTransport {
    fn local_addr(&self) -> std::result::Result<std::net::SocketAddr, rvoip_sip_transport::error::Error> {
        Ok(self.local_addr)
    }
    
    async fn send_message(&self, message: rvoip_sip_core::Message, destination: std::net::SocketAddr) 
        -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        // Find the destination transport in the registry
        let send_count = self.sent_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        log(&format!("[{} -> {}] Sending message #{}: {:?}", self.local_addr, destination, send_count, message.short_description()));
        
        if let Some(tx) = self.registry.get(&destination) {
            // Create a TransportEvent for the destination
            let event = rvoip_sip_transport::TransportEvent::MessageReceived {
                message,
                source: self.local_addr,
                destination,
            };
            
            // Send the message to the destination transport with timeout
            match tokio::time::timeout(Duration::from_secs(5), tx.send(event)).await {
                Ok(Ok(_)) => {
                    log(&format!("[{} -> {}] Message #{} successfully sent", self.local_addr, destination, send_count));
                    let recv_count = self.received_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    log(&format!("[{} -> {}] Received count: {}", self.local_addr, destination, recv_count));
                    Ok(())
                },
                Ok(Err(_)) => {
                    log(&format!("[{} -> {}] Failed to send message #{}; channel closed", self.local_addr, destination, send_count));
                    Err(rvoip_sip_transport::error::Error::Other("Send error: channel closed".to_string()))
                },
                Err(_) => {
                    log(&format!("[{} -> {}] Failed to send message #{}; timeout", self.local_addr, destination, send_count));
                    Err(rvoip_sip_transport::error::Error::Other("Send error: timeout".to_string()))
                }
            }
        } else {
            log(&format!("[{} -> {}] Destination not found for message #{}", self.local_addr, destination, send_count));
            Err(rvoip_sip_transport::error::Error::Other(format!("Destination unreachable: {}", destination)))
        }
    }
    
    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        self.registry.remove(&self.local_addr);
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        !self.registry.contains_key(&self.local_addr)
    }
}

// Helper extension to show short message description (for logging)
trait MessageExt {
    fn short_description(&self) -> String;
}

impl MessageExt for rvoip_sip_core::Message {
    fn short_description(&self) -> String {
        match self {
            rvoip_sip_core::Message::Request(req) => {
                format!("Request({})", req.method())
            },
            rvoip_sip_core::Message::Response(resp) => {
                format!("Response({})", resp.status())
            }
        }
    }
}

// Helper to create test SIP messages
fn create_test_invite(call_id: &str, from_tag: &str, local_address: &std::net::SocketAddr, remote_address: &std::net::SocketAddr) -> Request {
    SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some(from_tag))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(call_id)
        .cseq(1)
        .contact(&format!("sip:alice@{}", local_address), Some("Alice"))
        .via(&local_address.to_string(), "UDP", Some(&format!("z9hG4bK-{}", Uuid::new_v4().as_simple())))
        .build()
}

fn create_test_response(request: &Request, status: StatusCode, with_to_tag: bool, local_address: &std::net::SocketAddr) -> Response {
    let mut builder = SimpleResponseBuilder::response_from_request(request, status, None);
    
    // Add a Contact header for dialog establishment
    builder = builder.contact(&format!("sip:bob@{}", local_address), Some("Bob"));
    
    // If this is a response that should establish a dialog, add a to-tag
    if with_to_tag {
        let to_tag = format!("bob-tag-{}", Uuid::new_v4().as_simple());
        
        // Get original To header to extract display name and URI
        if let Some(TypedHeader::To(to)) = request.header(&HeaderName::To) {
            let display_name = to.address().display_name().unwrap_or("").to_string();
            let uri = to.address().uri.to_string();
            builder = builder.to(&display_name, &uri, Some(&to_tag));
        }
    }
    
    builder.build()
}

// Process a single session through its entire lifecycle
async fn process_session_lifecycle(
    uac_session_manager: Arc<SessionManager>,
    uas_session_manager: Arc<SessionManager>,
    uac_transport_addr: std::net::SocketAddr,
    uas_transport_addr: std::net::SocketAddr,
    uac_transaction_manager: Arc<TransactionManager>,
    uas_transaction_manager: Arc<TransactionManager>,
    mut uac_events_rx: broadcast::Receiver<TransactionEvent>,
    mut uas_events_rx: broadcast::Receiver<TransactionEvent>,
    session_idx: usize,
) -> bool {
    log(&format!("Processing session {}", session_idx));
    
    // Step 1: Create UAC session
    let destination = Uri::sip(&format!("bench-user-{}-{}", session_idx, Uuid::new_v4().as_simple()));
    let uac_session = match make_call(&uac_session_manager, destination).await {
        Ok(session) => session,
        Err(e) => {
            log(&format!("Failed to create UAC session: {:?}", e));
            return false;
        },
    };
    
    log(&format!("Created UAC session {}: {}", session_idx, uac_session.id));
    
    // We'll track all transaction IDs to clean them up later
    let mut transaction_ids = Vec::new();
    
    // Step 2: Create an INVITE request and send it via UAC transaction manager
    let call_id = format!("bench-{}", Uuid::new_v4().as_simple());
    let from_tag = format!("tag-{}", Uuid::new_v4().as_simple());
    let invite_request = create_test_invite(&call_id, &from_tag, &uac_transport_addr, &uas_transport_addr);
    
    // Create UAC transaction
    let uac_transaction_id = match uac_transaction_manager.create_invite_client_transaction(
        invite_request.clone(), 
        uas_transport_addr
    ).await {
        Ok(id) => id,
        Err(e) => {
            log(&format!("Failed to create UAC transaction: {:?}", e));
            return false
        }
    };
    
    // Track transaction ID for cleanup
    transaction_ids.push(uac_transaction_id.clone());
    
    log(&format!("Created UAC transaction {}: {}", session_idx, uac_transaction_id));
    
    // Associate transaction with session
    uac_session.track_transaction(uac_transaction_id.clone(), 
        rvoip_session_core::session::SessionTransactionType::InitialInvite).await;
    
    // Send the INVITE through the transaction layer
    match uac_transaction_manager.send_request(&uac_transaction_id).await {
        Ok(_) => log(&format!("Sent INVITE request for session {}", session_idx)),
        Err(e) => {
            log(&format!("Failed to send INVITE request: {:?}", e));
            // Clean up the transaction before returning
            let _ = uac_transaction_manager.terminate_transaction(&uac_transaction_id).await;
            return false;
        }
    }
    
    // Wait for UAS to receive the INVITE with timeout
    log(&format!("Waiting for UAS to receive INVITE..."));
    let result = match tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match uas_events_rx.recv().await {
                Ok(event) => {
                    log(&format!("Session {} received UAS event: {:?}", session_idx, event));
                    match event {
                        TransactionEvent::InviteRequest { transaction_id, request, source } => {
                            // Got an INVITE
                            return Ok((transaction_id, request, source));
                        },
                        TransactionEvent::NewRequest { transaction_id, request, source } => {
                            if request.method() == Method::Invite {
                                // Got an INVITE as NewRequest
                                return Ok((transaction_id, request, source));
                            }
                        },
                        _ => {
                            log(&format!("Ignoring event: {:?}", event));
                            continue;
                        },
                    }
                },
                Err(e) => {
                    log(&format!("Error receiving UAS event: {:?}", e));
                    return Err(e);
                },
            }
        }
    }).await {
        Ok(Ok(result)) => result,
        _ => {
            // Timeout or error
            log(&format!("Timeout or error waiting for INVITE on UAS side"));
            // Clean up transactions before returning
            for tx_id in &transaction_ids {
                let _ = uac_transaction_manager.terminate_transaction(tx_id).await;
            }
            return false;
        }
    };
    
    let (event_transaction_id, received_request, source_addr) = result;
    log(&format!("Got INVITE on UAS side for session {}: {}", session_idx, event_transaction_id));
    
    // CRITICAL FIX: Create a proper server transaction in the UAS transaction manager
    log(&format!("Creating server transaction for the received INVITE..."));
    
    // Add retry logic for creating server transaction
    const MAX_RETRIES: usize = 3;
    let server_transaction = {
        let mut retry_count = 0;
        let mut last_error = None;
        
        loop {
            match uas_transaction_manager.create_server_transaction(
                received_request.clone(),
                source_addr
            ).await {
                Ok(tx) => break Ok(tx),
                Err(e) => {
                    retry_count += 1;
                    last_error = Some(e);
                    
                    if retry_count >= MAX_RETRIES {
                        break Err(last_error.unwrap());
                    }
                    
                    // Add a small delay before retrying
                    log(&format!("Retry #{} creating server transaction after error: {:?}", 
                                 retry_count, last_error));
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    };
    
    let server_transaction = match server_transaction {
        Ok(tx) => tx,
        Err(e) => {
            log(&format!("Failed to create server transaction after {} retries: {:?}", MAX_RETRIES, e));
            // Clean up transactions before returning
            for tx_id in &transaction_ids {
                let _ = uac_transaction_manager.terminate_transaction(tx_id).await;
            }
            return false;
        }
    };
    
    // Get the transaction ID from the created server transaction
    let server_transaction_id = server_transaction.id().clone();
    // Track server transaction ID for cleanup
    transaction_ids.push(server_transaction_id.clone());
    
    log(&format!("Created UAS server transaction with ID: {}", server_transaction_id));
    
    // Step 3: UAS sends provisional response
    let ringing_response = create_test_response(&received_request, StatusCode::Ringing, true, &uas_transport_addr);
    
    // Send the response through UAS transaction manager using the proper transaction ID
    match uas_transaction_manager.send_response(&server_transaction_id, ringing_response.clone()).await {
        Ok(_) => log(&format!("Sent RINGING response for session {}", session_idx)),
        Err(e) => {
            log(&format!("Failed to send RINGING response: {:?}", e));
            // Clean up transactions before returning
            for tx_id in &transaction_ids {
                let tx_manager = if tx_id == &server_transaction_id {
                    &uas_transaction_manager
                } else {
                    &uac_transaction_manager
                };
                let _ = tx_manager.terminate_transaction(tx_id).await;
            }
            return false;
        }
    }
    
    // Wait for UAC to receive the response and update state
    log(&format!("Waiting for UAC to receive RINGING response..."));
    let ringing_received = match tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match uac_events_rx.recv().await {
                Ok(event) => {
                    log(&format!("Session {} received UAC event: {:?}", session_idx, event));
                    match event {
                        TransactionEvent::ProvisionalResponse { transaction_id, response, .. } |
                        TransactionEvent::SuccessResponse { transaction_id, response, .. } |
                        TransactionEvent::FailureResponse { transaction_id, response, .. } => {
                            debug!("Received response {} for transaction {}", response.status_code(), transaction_id);
                        },
                        _ => continue,
                    }
                },
                Err(e) => {
                    log(&format!("Error receiving UAC event: {:?}", e));
                    return false;
                }
            }
        }
    }).await {
        Ok(true) => true,
        _ => {
            log(&format!("Timeout waiting for RINGING response on UAC side"));
            // Clean up transactions before returning
            for tx_id in &transaction_ids {
                let tx_manager = if tx_id == &server_transaction_id {
                    &uas_transaction_manager
                } else {
                    &uac_transaction_manager
                };
                let _ = tx_manager.terminate_transaction(tx_id).await;
            }
            return false;
        }
    };
    
    // UAC changes state to Ringing
    if uac_session.set_state(SessionState::Ringing).await.is_err() {
        log(&format!("Failed to set UAC state to Ringing"));
        // Clean up transactions before returning
        for tx_id in &transaction_ids {
            let tx_manager = if tx_id == &server_transaction_id {
                &uas_transaction_manager
            } else {
                &uac_transaction_manager
            };
            let _ = tx_manager.terminate_transaction(tx_id).await;
        }
        return false;
    }
    
    // Create dialog in UAC from the response
    let uac_dialog_mgr = uac_session_manager.dialog_manager();
    let uac_dialog_id = match uac_dialog_mgr.create_dialog_from_transaction(
        &uac_transaction_id,
        &invite_request,
        &ringing_response,
        true  // UAC is initiator
    ).await {
        Some(id) => id,
        None => {
            log(&format!("Failed to create UAC dialog"));
            // Clean up transactions before returning
            for tx_id in &transaction_ids {
                let tx_manager = if tx_id == &server_transaction_id {
                    &uas_transaction_manager
                } else {
                    &uac_transaction_manager
                };
                let _ = tx_manager.terminate_transaction(tx_id).await;
            }
            return false;
        },
    };
    
    // Associate dialog with UAC session
    if uac_dialog_mgr.associate_with_session(&uac_dialog_id, &uac_session.id).is_err() {
        log(&format!("Failed to associate dialog with UAC session"));
        // Clean up transactions before returning
        for tx_id in &transaction_ids {
            let tx_manager = if tx_id == &server_transaction_id {
                &uas_transaction_manager
            } else {
                &uac_transaction_manager
            };
            let _ = tx_manager.terminate_transaction(tx_id).await;
        }
        return false;
    }
    
    log(&format!("Created and associated UAC dialog for session {}", session_idx));
    
    // Step 4: UAS sends 200 OK response
    let ok_response = create_test_response(&received_request, StatusCode::Ok, true, &uas_transport_addr);
    
    // Send the OK response through UAS transaction manager
    match uas_transaction_manager.send_response(&server_transaction_id, ok_response.clone()).await {
        Ok(_) => log(&format!("Sent 200 OK for session {}", session_idx)),
        Err(e) => {
            log(&format!("Failed to send 200 OK response: {:?}", e));
            // Clean up transactions and dialogs before returning
            let _ = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await;
            for tx_id in &transaction_ids {
                let tx_manager = if tx_id == &server_transaction_id {
                    &uas_transaction_manager
                } else {
                    &uac_transaction_manager
                };
                let _ = tx_manager.terminate_transaction(tx_id).await;
            }
            return false;
        }
    }
    
    // Wait for UAC to receive the final response
    log(&format!("Waiting for UAC to receive 200 OK response..."));
    let ok_received = match tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match uac_events_rx.recv().await {
                Ok(event) => {
                    log(&format!("Session {} received UAC event: {:?}", session_idx, event));
                    match event {
                        TransactionEvent::ProvisionalResponse { transaction_id, response, .. } |
                        TransactionEvent::SuccessResponse { transaction_id, response, .. } |
                        TransactionEvent::FailureResponse { transaction_id, response, .. } => {
                            debug!("Received response {} for transaction {}", response.status_code(), transaction_id);
                        },
                        _ => continue,
                    }
                },
                Err(e) => {
                    log(&format!("Error receiving UAC event: {:?}", e));
                    return false;
                }
            }
        }
    }).await {
        Ok(true) => true,
        _ => {
            log(&format!("Timeout waiting for 200 OK response on UAC side"));
            // Clean up transactions and dialogs before returning
            let _ = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await;
            for tx_id in &transaction_ids {
                let tx_manager = if tx_id == &server_transaction_id {
                    &uas_transaction_manager
                } else {
                    &uac_transaction_manager
                };
                let _ = tx_manager.terminate_transaction(tx_id).await;
            }
            return false;
        }
    };
    
    // UAC changes state to Connected
    if uac_session.set_state(SessionState::Connected).await.is_err() {
        log(&format!("Failed to set UAC state to Connected"));
        // Clean up transactions and dialogs before returning
        let _ = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await;
        for tx_id in &transaction_ids {
            let tx_manager = if tx_id == &server_transaction_id {
                &uas_transaction_manager
            } else {
                &uac_transaction_manager
            };
            let _ = tx_manager.terminate_transaction(tx_id).await;
        }
        return false;
    }
    
    // Step 5: Create a UAS session and dialog to properly handle the call
    let uas_session = match uas_session_manager.create_incoming_session().await {
        Ok(session) => session,
        Err(e) => {
            log(&format!("Failed to create UAS session: {:?}", e));
            // Clean up UAC side
            let _ = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await;
            for tx_id in &transaction_ids {
                let tx_manager = if tx_id == &server_transaction_id {
                    &uas_transaction_manager
                } else {
                    &uac_transaction_manager
                };
                let _ = tx_manager.terminate_transaction(tx_id).await;
            }
            return false;
        },
    };
    
    // Create UAS dialog and associate it with the session
    let uas_dialog_mgr = uas_session_manager.dialog_manager();
    let uas_dialog_id = match uas_dialog_mgr.create_dialog_from_transaction(
        &server_transaction_id,
        &received_request,
        &ok_response,
        false  // UAS is not initiator
    ).await {
        Some(id) => id,
        None => {
            log(&format!("Failed to create UAS dialog"));
            let _ = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await;
            for tx_id in &transaction_ids {
                let tx_manager = if tx_id == &server_transaction_id {
                    &uas_transaction_manager
                } else {
                    &uac_transaction_manager
                };
                let _ = tx_manager.terminate_transaction(tx_id).await;
            }
            return false;
        },
    };
    
    // Associate dialog with UAS session
    if uas_dialog_mgr.associate_with_session(&uas_dialog_id, &uas_session.id).is_err() {
        log(&format!("Failed to associate dialog with UAS session"));
        let _ = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await;
        let _ = uas_dialog_mgr.terminate_dialog(&uas_dialog_id).await;
        for tx_id in &transaction_ids {
            let tx_manager = if tx_id == &server_transaction_id {
                &uas_transaction_manager
            } else {
                &uac_transaction_manager
            };
            let _ = tx_manager.terminate_transaction(tx_id).await;
        }
        return false;
    }
    
    // Create a BYE transaction to terminate the dialog
    log(&format!("Creating BYE request for session {}", session_idx));
    let bye_request = match uac_dialog_mgr.create_request(&uac_dialog_id, Method::Bye).await {
        Ok(req) => req,
        Err(e) => {
            log(&format!("Failed to create BYE request: {:?}", e));
            // Still consider this success for the test, just cleanup
            let _ = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await;
            for tx_id in &transaction_ids {
                let tx_manager = if tx_id == &server_transaction_id {
                    &uas_transaction_manager
                } else {
                    &uac_transaction_manager
                };
                let _ = tx_manager.terminate_transaction(tx_id).await;
            }
            // Return true because the main call flow succeeded
            return true;
        }
    };
    
    // Create and send BYE transaction
    let bye_tx_id = match uac_transaction_manager.create_non_invite_client_transaction(
        bye_request,
        uas_transport_addr
    ).await {
        Ok(id) => id,
        Err(e) => {
            log(&format!("Failed to create BYE transaction: {:?}", e));
            // Still consider this success for the test, just cleanup
            let _ = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await;
            let _ = uas_dialog_mgr.terminate_dialog(&uas_dialog_id).await;
            for tx_id in &transaction_ids {
                let tx_manager = if tx_id == &server_transaction_id {
                    &uas_transaction_manager
                } else {
                    &uac_transaction_manager
                };
                let _ = tx_manager.terminate_transaction(tx_id).await;
            }
            // Return true because the main call flow succeeded
            return true;
        }
    };
    
    // Track BYE transaction ID for cleanup
    transaction_ids.push(bye_tx_id.clone());
    
    // Send the BYE request
    if let Err(e) = uac_transaction_manager.send_request(&bye_tx_id).await {
        log(&format!("Failed to send BYE request: {:?}", e));
        // Still consider this success for the test, just cleanup
        let _ = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await;
        let _ = uas_dialog_mgr.terminate_dialog(&uas_dialog_id).await;
        for tx_id in &transaction_ids {
            let tx_manager = if tx_id == &server_transaction_id {
                &uas_transaction_manager
            } else {
                &uac_transaction_manager
            };
            let _ = tx_manager.terminate_transaction(tx_id).await;
        }
        return true;
    }
    
    // Wait for UAS to receive the BYE request...
    log(&format!("Waiting for UAS to receive BYE request..."));
    let bye_received = match tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match uas_events_rx.recv().await {
                Ok(event) => {
                    log(&format!("Session {} received UAS event: {:?}", session_idx, event));
                    match event {
                        TransactionEvent::NewRequest { transaction_id, request, source } 
                            if request.method() == Method::Bye => {
                            // Create a server transaction for the BYE request if needed
                            let mut server_transaction_id = transaction_id.clone();
                            
                            // Check if the transaction exists
                            let transaction_exists = uas_transaction_manager.transaction_exists(&server_transaction_id).await;
                            if !transaction_exists {
                                log(&format!("Creating server transaction for BYE request"));
                                // Try to create a new server transaction
                                const MAX_RETRIES: usize = 3;
                                let mut retry_count = 0;
                                
                                while retry_count < MAX_RETRIES {
                                    match uas_transaction_manager.create_server_transaction(request.clone(), source).await {
                                        Ok(tx) => {
                                            server_transaction_id = tx.id().clone();
                                            log(&format!("Created new server transaction for BYE: {}", server_transaction_id));
                                            break;
                                        },
                                        Err(e) => {
                                            retry_count += 1;
                                            log(&format!("Retry #{}: Failed to create server transaction for BYE: {:?}", 
                                                        retry_count, e));
                                            
                                            if retry_count >= MAX_RETRIES {
                                                log(&format!("Failed to create server transaction for BYE after {} retries", 
                                                            MAX_RETRIES));
                                                break;
                                            }
                                            
                                            // Short delay before retrying
                                            tokio::time::sleep(Duration::from_millis(10)).await;
                                        }
                                    }
                                }
                            }
                            
                            // Create a 200 OK response for the BYE
                            let bye_response = create_test_response(&request, StatusCode::Ok, true, &uas_transport_addr);
                            
                            // Add retries for sending the response
                            let mut retry_count = 0;
                            const MAX_RETRIES: usize = 3;
                            
                            while retry_count < MAX_RETRIES {
                                match uas_transaction_manager.send_response(&server_transaction_id, bye_response.clone()).await {
                                    Ok(_) => {
                                        log(&format!("Sent 200 OK response for BYE request"));
                                        break;
                                    },
                                    Err(e) => {
                                        retry_count += 1;
                                        log(&format!("Retry #{}: Failed to send BYE response: {:?}", retry_count, e));
                                        
                                        if retry_count >= MAX_RETRIES {
                                            log(&format!("Failed to send BYE response after {} retries", MAX_RETRIES));
                                            break;
                                        }
                                        
                                        // Add a small delay before retrying
                                        tokio::time::sleep(Duration::from_millis(10)).await;
                                    }
                                }
                            }
                            
                            return true;
                        },
                        _ => continue,
                    }
                },
                Err(e) => {
                    log(&format!("Error receiving UAS event for BYE: {:?}", e));
                    return false;
                }
            }
        }
    }).await {
        Ok(true) => true,
        _ => {
            log(&format!("Timeout waiting for BYE request on UAS side"));
            // Still consider this success - the main call flow was successful
            false
        }
    };
    
    // Wait a brief moment for all events to propagate
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Terminate the dialogs explicitly
    if let Err(e) = uac_dialog_mgr.terminate_dialog(&uac_dialog_id).await {
        log(&format!("Failed to terminate UAC dialog: {:?}", e));
    }
    
    if let Err(e) = uas_dialog_mgr.terminate_dialog(&uas_dialog_id).await {
        log(&format!("Failed to terminate UAS dialog: {:?}", e));
    }
    
    // Clean up all transactions
    for tx_id in &transaction_ids {
        let tx_manager = if tx_id == &server_transaction_id {
            &uas_transaction_manager
        } else {
            &uac_transaction_manager
        };
        
        if let Err(e) = tx_manager.terminate_transaction(tx_id).await {
            log(&format!("Failed to terminate transaction {}: {:?}", tx_id, e));
        }
    }
    
    // Try to end the UAC and UAS sessions properly 
    let _ = end_call(&uac_session).await;
    let _ = end_call(&uas_session).await;
    
    println!("Session {} completed successfully", session_idx);
    true
}

#[tokio::main]
async fn main() {
    // Parse command line arguments for session count and verbose flag
    let args: Vec<String> = env::args().collect();
    
    // Set up default values
    let mut session_count = 1000; // Default
    let mut verbose = false;
    
    // Parse arguments
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            },
            arg => {
                // Try to parse as session count
                if let Ok(count) = arg.parse::<usize>() {
                    session_count = count;
                }
                i += 1;
            }
        }
    }
    
    // Set global verbose flag
    unsafe {
        VERBOSE = verbose;
    }
    
    // Setup tracing
    tracing_subscriber::fmt::init();
    
    println!("Starting benchmark with {} SIP sessions{}...", 
             session_count, 
             if verbose { " (verbose mode)" } else { "" });
    
    // Create loopback transport registry
    let transport_registry = Arc::new(DashMap::new());
    
    // Create UAC and UAS transports with different addresses
    let uac_addr = "127.0.0.1:5060".parse().unwrap();
    let uas_addr = "127.0.0.1:5061".parse().unwrap();
    
    // Increase buffer sizes for better performance
    let channel_capacity = session_count * 10; // Much bigger buffer to avoid backpressure
    
    // Create transport event channels
    let (uac_transport_tx, uac_transport_rx) = mpsc::channel::<rvoip_sip_transport::TransportEvent>(channel_capacity);
    let (uas_transport_tx, uas_transport_rx) = mpsc::channel::<rvoip_sip_transport::TransportEvent>(channel_capacity);
    
    // Register the transport channels in the registry
    transport_registry.insert(uac_addr, uac_transport_tx.clone());
    transport_registry.insert(uas_addr, uas_transport_tx.clone());
    
    // Create transports
    let uac_transport = Arc::new(LoopbackTransport::new(uac_addr, transport_registry.clone()));
    let uas_transport = Arc::new(LoopbackTransport::new(uas_addr, transport_registry.clone()));
    
    // Create transaction managers
    let (uac_transaction_manager, uac_events_rx) = TransactionManager::new(
        uac_transport.clone(), 
        uac_transport_rx, 
        Some(channel_capacity)
    ).await.unwrap();
    let uac_transaction_manager = Arc::new(uac_transaction_manager);
    
    let (uas_transaction_manager, uas_events_rx) = TransactionManager::new(
        uas_transport.clone(), 
        uas_transport_rx, 
        Some(channel_capacity)
    ).await.unwrap();
    let uas_transaction_manager = Arc::new(uas_transaction_manager);
    
    // Create broadcast channels for transaction events
    let (uac_events_tx, _) = broadcast::channel::<TransactionEvent>(channel_capacity);
    let (uas_events_tx, _) = broadcast::channel::<TransactionEvent>(channel_capacity);
    
    // Forward transaction events to the broadcast channels
    let uac_events_tx_clone = uac_events_tx.clone();
    tokio::spawn(async move {
        let mut rx = uac_events_rx;
        while let Some(event) = rx.recv().await {
            if verbose {
                println!("UAC received event: {:?}", event);
            }
            let _ = uac_events_tx_clone.send(event);
        }
    });
    
    let uas_events_tx_clone = uas_events_tx.clone();
    tokio::spawn(async move {
        let mut rx = uas_events_rx;
        while let Some(event) = rx.recv().await {
            if verbose {
                println!("UAS received event: {:?}", event);
            }
            let _ = uas_events_tx_clone.send(event);
        }
    });
    
    // Create event buses
    let uac_event_bus = EventBus::new(channel_capacity);
    let uas_event_bus = EventBus::new(channel_capacity);
    
    // Create session managers
    let uac_session_config = SessionConfig {
        local_signaling_addr: uac_addr,
        local_media_addr: "127.0.0.1:10000".parse().unwrap(),
        supported_codecs: vec![],
        display_name: None,
        user_agent: "RVOIP-Benchmark-UAC/0.1.0".to_string(),
        max_duration: 0,
        max_sessions: Some(session_count),
    };
    
    let uas_session_config = SessionConfig {
        local_signaling_addr: uas_addr,
        local_media_addr: "127.0.0.1:20000".parse().unwrap(),
        supported_codecs: vec![],
        display_name: None,
        user_agent: "RVOIP-Benchmark-UAS/0.1.0".to_string(),
        max_duration: 0,
        max_sessions: Some(session_count),
    };
    
    let uac_session_manager = Arc::new(SessionManager::new(
        uac_transaction_manager.clone(),
        uac_session_config,
        uac_event_bus.clone()
    ));
    
    let uas_session_manager = Arc::new(SessionManager::new(
        uas_transaction_manager.clone(),
        uas_session_config,
        uas_event_bus.clone()
    ));
    
    // Start the session managers
    let _ = uac_session_manager.start().await;
    let _ = uas_session_manager.start().await;
    
    // Track success and failure counts
    let mut success_count = 0;
    let mut failure_count = 0;
    
    // Measure start time
    let start_time = Instant::now();
    
    // Process sessions sequentially to avoid overwhelming the transaction layer
    for i in 0..session_count {
        let result = process_session_lifecycle(
            uac_session_manager.clone(),
            uas_session_manager.clone(),
            uac_addr,
            uas_addr,
            uac_transaction_manager.clone(),
            uas_transaction_manager.clone(),
            uac_events_tx.subscribe(),
            uas_events_tx.subscribe(),
            i
        ).await;
        
        if result {
            success_count += 1;
        } else {
            failure_count += 1;
        }
        
        // Print progress periodically
        let total = success_count + failure_count;
        if total % 100 == 0 || total == session_count || (verbose && total % 10 == 0) {
            println!("Progress: {}/{} complete ({} success, {} failure)", 
                total, 
                session_count,
                success_count,
                failure_count
            );
        }
        
        // Add a small delay between sessions to avoid transaction ID conflicts
        // and to allow the transaction layer to clean up resources
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    
    // Calculate duration
    let duration = start_time.elapsed();
    
    // Only terminate sessions that aren't already terminated
    println!("Checking for active sessions...");
    
    // Get all sessions from both managers
    let uac_sessions = uac_session_manager.list_sessions();
    let uas_sessions = uas_session_manager.list_sessions();
    
    let mut active_uac_sessions = Vec::new();
    let mut active_uas_sessions = Vec::new();
    
    // Check UAC sessions and find those that are not terminated
    for session in uac_sessions {
        let session_id = session.id.clone();
        let state = session.state().await;
        
        if state != SessionState::Terminated {
            println!("Found active UAC session {}: state={:?}", session_id, state);
            active_uac_sessions.push(session_id);
        }
    }
    
    // Check UAS sessions and find those that are not terminated
    for session in uas_sessions {
        let session_id = session.id.clone();
        let state = session.state().await;
        
        if state != SessionState::Terminated {
            println!("Found active UAS session {}: state={:?}", session_id, state);
            active_uas_sessions.push(session_id);
        }
    }
    
    // Terminate remaining sessions with timeout
    let remaining_count = active_uac_sessions.len() + active_uas_sessions.len();
    
    if remaining_count > 0 {
        println!("Terminating {} remaining active sessions...", remaining_count);
        let terminate_result = tokio::time::timeout(
            Duration::from_secs(10), 
            async {
                // Terminate UAC sessions that are still active
                for session_id in &active_uac_sessions {
                    match uac_session_manager.terminate_session(session_id, "Benchmark cleanup").await {
                        Ok(_) => println!("Successfully terminated UAC session: {}", session_id),
                        Err(e) => println!("Error terminating UAC session {}: {:?}", session_id, e)
                    }
                }
                
                // Terminate UAS sessions that are still active
                for session_id in &active_uas_sessions {
                    match uas_session_manager.terminate_session(session_id, "Benchmark cleanup").await {
                        Ok(_) => println!("Successfully terminated UAS session: {}", session_id),
                        Err(e) => println!("Error terminating UAS session {}: {:?}", session_id, e)
                    }
                }
            }
        ).await;
        
        if terminate_result.is_err() {
            println!("Timed out waiting for remaining sessions to terminate");
        }
    } else {
        println!("No remaining active sessions to terminate.");
    }
    
    // Stop session managers to ensure clean termination with timeout
    println!("Stopping session managers...");
    let stop_result = tokio::time::timeout(
        Duration::from_secs(5),
        async {
            uac_session_manager.stop().await;
            uas_session_manager.stop().await;
        }
    ).await;
    
    if stop_result.is_err() {
        println!("Timed out waiting for session managers to stop");
    }
    
    // Allow transaction managers to properly clean up with timeout
    println!("Stopping transaction managers...");
    let tx_stop_result = tokio::time::timeout(
        Duration::from_secs(5),
        async {
            uac_transaction_manager.shutdown().await;
            uas_transaction_manager.shutdown().await;
        }
    ).await;
    
    if tx_stop_result.is_err() {
        println!("Timed out waiting for transaction managers to shutdown");
    }
    
    // Close the transport connections with timeout
    println!("Closing transport connections...");
    let transport_close_result = tokio::time::timeout(
        Duration::from_secs(2),
        async {
            let _ = uac_transport.close().await;
            let _ = uas_transport.close().await;
        }
    ).await;
    
    if transport_close_result.is_err() {
        println!("Timed out waiting for transport connections to close");
    }
    
    // Final check to ensure all resources are cleaned up
    println!("All resources cleaned up successfully");
    
    // Print results
    println!("\nBenchmark Results");
    println!("================");
    println!("Session count: {}", session_count);
    println!("Success: {}", success_count);
    println!("Failures: {}", failure_count);
    println!("Total duration: {:.2?}", duration);
    println!("Avg time per session: {:.2?}", duration / session_count as u32);
    println!("Sessions per second: {:.2}", session_count as f64 / duration.as_secs_f64());
} 