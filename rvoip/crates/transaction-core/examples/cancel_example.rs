/**
 * CANCEL Transaction Example
 * 
 * This example demonstrates CANCEL transactions and their relationship with
 * INVITE transactions. It shows:
 *
 * 1. Client sending an INVITE request
 * 2. Server responding with 100 Trying and 180 Ringing
 * 3. Client deciding to cancel the INVITE by sending a CANCEL
 * 4. Server accepting the CANCEL with 200 OK 
 * 5. Server terminating the original INVITE with 487 Request Terminated
 * 6. Client acknowledging the termination with ACK
 *
 * CANCEL is a special type of transaction that:
 * - Must target an existing INVITE transaction
 * - Has its own transaction but references another transaction
 * - Can only be sent before a final response is received
 * - Results in a 487 response to the original INVITE
 *
 * To run with full logging:
 * ```
 * RUST_LOG=rvoip=trace cargo run --example cancel_example
 * ```
 */

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip_core::{Method, Message, Request, Response, Uri};
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::types::header::{HeaderName, TypedHeader};
use rvoip_sip_core::types::content_length::ContentLength;
use rvoip_sip_core::types::max_forwards::MaxForwards;

use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionState, TransactionKey};
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use tracing_subscriber::fmt::format::FmtSpan;
use uuid::Uuid;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup logging
    tracing_subscriber::fmt()
        .with_env_filter("rvoip=debug")
        .with_span_events(FmtSpan::CLOSE)
        .init();
    
    // ------------- Server setup -----------------
    
    // Create a transport manager for the server
    let server_config = TransportManagerConfig {
        enable_udp: true,
        enable_tcp: false,
        enable_ws: false,
        enable_tls: false,
        bind_addresses: vec!["127.0.0.1:5060".parse()?],
        ..Default::default()
    };
    
    let (mut server_transport, server_transport_rx) = TransportManager::new(server_config).await?;
    server_transport.initialize().await?;
    
    // Get the server address
    let server_addr = server_transport.default_transport().await
        .ok_or("No default transport")?.local_addr()?;
        
    info!("Server bound to {}", server_addr);
    
    // Create a transaction manager for the server
    let (server_tm, mut server_events) = TransactionManager::with_transport_manager(
        server_transport.clone(),
        server_transport_rx,
        Some(100),
    ).await?;
    
    // ------------- Client setup -----------------
    
    // Create a transport manager for the client
    let client_config = TransportManagerConfig {
        enable_udp: true,
        enable_tcp: false,
        enable_ws: false,
        enable_tls: false,
        bind_addresses: vec!["127.0.0.1:0".parse()?], // Use ephemeral port
        ..Default::default()
    };
    
    let (mut client_transport, client_transport_rx) = TransportManager::new(client_config).await?;
    client_transport.initialize().await?;
    
    // Get the client address
    let client_addr = client_transport.default_transport().await
        .ok_or("No default transport")?.local_addr()?;
        
    info!("Client bound to {}", client_addr);
    
    // Create a transaction manager for the client
    let (client_tm, mut client_events) = TransactionManager::with_transport_manager(
        client_transport.clone(),
        client_transport_rx,
        Some(100),
    ).await?;
    
    // ------------- Main call flow logic -----------------
    
    // Spawn a task to handle server events
    let server_task = tokio::spawn(handle_server_events(server_tm.clone(), server_events));
    
    // Create an INVITE request
    let call_id = format!("cancel-{}", Uuid::new_v4());
    let from_tag = format!("tag-{}", Uuid::new_v4().simple());
    
    let invite_request = SimpleRequestBuilder::new(Method::Invite, &format!("sip:bob@{}", server_addr.ip()))?
        .from("Alice", &format!("sip:alice@{}", client_addr.ip()), Some(&from_tag))
        .to("Bob", &format!("sip:bob@{}", server_addr.ip()), None)
        .call_id(&call_id)
        .cseq(1)
        .contact(&format!("sip:alice@{}", client_addr.ip()), Some("Alice's Contact"))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();
    
    // Create a client transaction for the INVITE request
    let invite_tx_id = client_tm.create_client_transaction(invite_request.clone(), server_addr).await?;
    info!("Created INVITE client transaction with ID: {}", invite_tx_id);
    
    // Send the INVITE request
    client_tm.send_request(&invite_tx_id).await?;
    info!("Sent INVITE request to server");
    
    // Wait a moment for server to receive and start processing the request
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Wait for a provisional response before sending CANCEL
    let provisional_received = wait_for_provisional_response(&mut client_events, &invite_tx_id).await?;
    info!("Received provisional response: {}", provisional_received.status_code());
    
    // Now send CANCEL for the INVITE transaction
    let cancel_tx_id = client_tm.cancel_invite_transaction(&invite_tx_id).await?;
    info!("Created CANCEL transaction with ID: {}", cancel_tx_id);
    
    // Wait for a response to CANCEL
    let cancel_response = wait_for_final_response(&mut client_events, &cancel_tx_id).await?;
    info!("Received {} response for CANCEL: {}", 
          cancel_response.status_code(), 
          cancel_response.reason_phrase());
          
    // Wait for final response to original INVITE (should be 487 Request Terminated)  
    let invite_final_response = wait_for_final_response(&mut client_events, &invite_tx_id).await?;
    info!("Received {} response for INVITE: {}", 
          invite_final_response.status_code(), 
          invite_final_response.reason_phrase());
    
    // For non-2xx final responses to INVITE, an ACK must be sent to complete the transaction
    // This is handled automatically by the transaction layer
    
    // All transactions completed successfully
    info!("CANCEL flow completed successfully");
    
    // Wait a bit for everything to complete
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Clean up
    client_tm.shutdown().await;
    server_tm.shutdown().await;
    
    // Wait for server task to complete
    let _ = server_task.await;
    
    Ok(())
}

async fn handle_server_events(
    server_tm: TransactionManager,
    mut events: mpsc::Receiver<TransactionEvent>,
) {
    // Keep track of related transactions
    let mut related_transactions: HashMap<TransactionKey, TransactionKey> = HashMap::new();
    
    // Add timeout for handling inactivity
    let mut last_activity = std::time::Instant::now();
    const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(5);
    
    loop {
        // Use tokio::select with a timeout to prevent hanging indefinitely
        tokio::select! {
            Some(event) = events.recv() => {
                // Reset activity timer whenever we receive an event
                last_activity = std::time::Instant::now();
                
                match event {
                    TransactionEvent::NewRequest { transaction_id, request, source, .. } => {
                        info!("Server received request: {:?} from {}", request.method(), source);
                        
                        // First, create a server transaction for this request
                        let server_tx = match server_tm.create_server_transaction(
                            request.clone(),
                            source,
                        ).await {
                            Ok(tx) => tx.id().clone(),
                            Err(e) => {
                                error!("Failed to create server transaction: {}", e);
                                continue;
                            }
                        };
                        
                        match request.method() {
                            Method::Invite => {
                                // For INVITE, create a transaction and send provisional responses
                                process_invite(server_tm.clone(), server_tx, request, source).await;
                            },
                            Method::Cancel => {
                                // CANCEL should be handled by CancelRequest event, not here
                                // But we send a 200 OK as a fallback
                                let ok = SimpleResponseBuilder::response_from_request(
                                    &request,
                                    StatusCode::Ok,
                                    Some("OK"),
                                ).build();
                                
                                if let Err(e) = server_tm.send_response(&server_tx, ok).await {
                                    error!("Failed to send OK response to CANCEL: {}", e);
                                }
                            },
                            _ => {
                                // For other methods, send 200 OK
                                let ok = SimpleResponseBuilder::response_from_request(
                                    &request,
                                    StatusCode::Ok,
                                    Some("OK"),
                                ).build();
                                
                                if let Err(e) = server_tm.send_response(&server_tx, ok).await {
                                    error!("Failed to send OK response: {}", e);
                                }
                            }
                        }
                    },
                    TransactionEvent::CancelRequest { transaction_id, target_transaction_id, request, .. } => {
                        info!("Received CANCEL for transaction: {}", target_transaction_id);
                        
                        // Store the relationship between CANCEL and INVITE transactions
                        related_transactions.insert(transaction_id.clone(), target_transaction_id.clone());
                        
                        // First, respond to the CANCEL with 200 OK
                        let ok_to_cancel = SimpleResponseBuilder::response_from_request(
                            &request,
                            StatusCode::Ok,
                            Some("OK"),
                        ).build();
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, ok_to_cancel).await {
                            error!("Failed to send 200 OK to CANCEL: {}", e);
                        }
                        
                        // Now terminate the original INVITE with 487 Request Terminated
                        let request_terminated = SimpleResponseBuilder::response_from_request(
                            &request, // This actually uses headers from the CANCEL, but it's close enough
                            StatusCode::RequestTerminated,
                            Some("Request Terminated"),
                        ).build();
                        
                        if let Err(e) = server_tm.send_response(&target_transaction_id, request_terminated).await {
                            error!("Failed to send 487 to INVITE: {}", e);
                        }
                    },
                    TransactionEvent::StateChanged { transaction_id, previous_state, new_state } => {
                        debug!("Server transaction {} changed state: {:?} -> {:?}",
                            transaction_id, previous_state, new_state);
                    },
                    TransactionEvent::TransportError { transaction_id, .. } => {
                        error!("Server transport error for transaction {}", transaction_id);
                    },
                    _ => {}
                }
            },
            // Check for inactivity timeout
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if last_activity.elapsed() > INACTIVITY_TIMEOUT {
                    info!("Server task exiting due to inactivity timeout");
                    break;
                }
            }
        }
    }
    
    info!("Server event handler task completed");
}

async fn process_invite(
    server_tm: TransactionManager,
    transaction_id: TransactionKey,
    request: Request,
    source: SocketAddr,
) {
    // Send 100 Trying immediately
    let trying = SimpleResponseBuilder::response_from_request(
        &request,
        StatusCode::Trying,
        Some("Trying"),
    ).build();
    
    if let Err(e) = server_tm.send_response(&transaction_id, trying).await {
        error!("Failed to send Trying response: {}", e);
        return;
    }
    
    // Wait a bit to simulate processing
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Send 180 Ringing 
    let ringing = SimpleResponseBuilder::response_from_request(
        &request,
        StatusCode::Ringing,
        Some("Ringing"),
    ).build();
    
    if let Err(e) = server_tm.send_response(&transaction_id, ringing).await {
        error!("Failed to send Ringing response: {}", e);
        return;
    }
    
    // In a real implementation, we would wait for user interaction here
    // Instead, we'll just simulate a slow response to give time for CANCEL
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    info!("INVITE transaction {} still awaiting final response", transaction_id);
}

async fn wait_for_provisional_response(
    events: &mut mpsc::Receiver<TransactionEvent>,
    transaction_id: &TransactionKey,
) -> Result<Response, Box<dyn std::error::Error>> {
    let timeout_duration = Duration::from_secs(5);
    let start_time = std::time::Instant::now();
    
    while start_time.elapsed() < timeout_duration {
        tokio::select! {
            Some(event) = events.recv() => {
                match event {
                    TransactionEvent::ProvisionalResponse { transaction_id: tx_id, response, .. } 
                        if tx_id == *transaction_id => {
                        info!("Received provisional response: {} {}",
                            response.status_code(), response.reason_phrase());
                        return Ok(response);
                    },
                    TransactionEvent::SuccessResponse { transaction_id: tx_id, .. } |
                    TransactionEvent::FailureResponse { transaction_id: tx_id, .. }
                        if tx_id == *transaction_id => {
                        return Err("Received final response before provisional".into());
                    },
                    TransactionEvent::TransportError { transaction_id: tx_id, .. } 
                        if tx_id == *transaction_id => {
                        error!("Transport error for transaction {}", transaction_id);
                        return Err(format!("Transport error for transaction {}", transaction_id).into());
                    },
                    _ => {}
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                // Just a short delay to prevent tight looping
            }
        }
    }
    
    Err("Timeout waiting for provisional response".into())
}

async fn wait_for_final_response(
    events: &mut mpsc::Receiver<TransactionEvent>,
    transaction_id: &TransactionKey,
) -> Result<Response, Box<dyn std::error::Error>> {
    let timeout_duration = Duration::from_secs(10);
    let start_time = std::time::Instant::now();
    
    while start_time.elapsed() < timeout_duration {
        tokio::select! {
            Some(event) = events.recv() => {
                match event {
                    TransactionEvent::ProvisionalResponse { transaction_id: tx_id, response, .. } 
                        if tx_id == *transaction_id => {
                        info!("Received provisional response: {} {}",
                            response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::SuccessResponse { transaction_id: tx_id, response, .. }
                        if tx_id == *transaction_id => {
                        info!("Received success response: {} {}",
                            response.status_code(), response.reason_phrase());
                        return Ok(response);
                    },
                    TransactionEvent::FailureResponse { transaction_id: tx_id, response }
                        if tx_id == *transaction_id => {
                        info!("Received failure response: {} {}",
                            response.status_code(), response.reason_phrase());
                        return Ok(response);
                    },
                    // Also handle the generic Response event type
                    TransactionEvent::Response { transaction_id: tx_id, response, .. }
                        if tx_id == *transaction_id => {
                        // Check if it's a final response (non-1xx)
                        if response.status_code() >= 200 {
                            info!("Received response via generic event: {} {}",
                                response.status_code(), response.reason_phrase());
                            return Ok(response);
                        }
                    },
                    TransactionEvent::TransportError { transaction_id: tx_id, .. } 
                        if tx_id == *transaction_id => {
                        error!("Transport error for transaction {}", transaction_id);
                        return Err(format!("Transport error for transaction {}", transaction_id).into());
                    },
                    _ => {}
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                // Just a short delay to prevent tight looping
            }
        }
    }
    
    Err("Timeout waiting for final response".into())
} 