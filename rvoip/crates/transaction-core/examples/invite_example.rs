/**
 * INVITE Transaction Example
 * 
 * This example demonstrates a complete INVITE transaction flow between
 * a SIP client and server using the transaction-core and sip-transport integration.
 * The example shows:
 *
 * 1. Client initiating an INVITE request to establish a call
 * 2. Server responding with 100 Trying and 180 Ringing provisional responses
 * 3. Server accepting the call with 200 OK
 * 4. Client acknowledging with ACK for the 200 OK
 * 5. Client terminating the call with BYE
 * 6. Server acknowledging the BYE with 200 OK
 *
 * This demonstrates all stages of a basic SIP call flow including proper
 * transaction state transitions and message handling.
 *
 * To run with full logging:
 * ```
 * RUST_LOG=rvoip=trace cargo run --example invite_example
 * ```
 */

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip_core::{Method, Message, Request, Response, Uri};
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::types::header::{HeaderName, TypedHeader};
use rvoip_sip_core::types::cseq::CSeq;
use rvoip_sip_core::types::content_length::ContentLength;
use rvoip_sip_core::types::max_forwards::MaxForwards;

use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionState, TransactionKey};
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use tracing_subscriber::fmt::format::FmtSpan;
use uuid::Uuid;

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
    tokio::spawn(handle_server_events(server_tm.clone(), server_events));
    
    // Create call-id and tags for dialog identification
    let call_id = format!("call-{}", Uuid::new_v4());
    let from_tag = format!("tag-{}", Uuid::new_v4().simple());
    
    // Create an INVITE request
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
    
    // Wait for the final response from the server
    let response_200_ok = wait_for_final_response(&mut client_events, &invite_tx_id).await?;
    info!("Received 200 OK for INVITE");
    
    // Now we need to send an ACK for the 200 OK
    // The ACK for a 2xx response is outside the original transaction
    let ack_request = client_tm.create_ack_for_2xx(&invite_tx_id, &response_200_ok).await?;
    client_tm.send_ack_for_2xx(&invite_tx_id, &response_200_ok).await?;
    info!("Sent ACK for 200 OK");
    
    // Sleep for a short period to simulate an active call
    info!("Call established, waiting for 2 seconds...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Now terminate the call with a BYE request
    let to_tag = response_200_ok.to()
        .and_then(|to| to.tag())
        .ok_or("Missing To tag in 200 OK response")?;
    
    let bye_request = SimpleRequestBuilder::new(Method::Bye, &format!("sip:bob@{}", server_addr.ip()))?
        .from("Alice", &format!("sip:alice@{}", client_addr.ip()), Some(&from_tag))
        .to("Bob", &format!("sip:bob@{}", server_addr.ip()), Some(to_tag))
        .call_id(&call_id)
        .cseq(2) // Increase CSeq for the next request in the dialog
        .contact(&format!("sip:alice@{}", client_addr.ip()), Some("Alice's Contact"))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();
    
    // Create a client transaction for the BYE request
    let bye_tx_id = client_tm.create_client_transaction(bye_request, server_addr).await?;
    info!("Created BYE client transaction with ID: {}", bye_tx_id);
    
    // Send the BYE request
    client_tm.send_request(&bye_tx_id).await?;
    info!("Sent BYE request to server");
    
    // Wait for the final response for the BYE
    let _bye_response = wait_for_final_response(&mut client_events, &bye_tx_id).await?;
    info!("Received 200 OK for BYE");
    
    // Call has been terminated successfully
    info!("Call terminated successfully");
    
    // Wait a bit for everything to complete
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Clean up
    client_tm.shutdown().await;
    server_tm.shutdown().await;
    
    Ok(())
}

async fn handle_server_events(
    server_tm: TransactionManager,
    mut events: mpsc::Receiver<TransactionEvent>,
) {
    let mut active_dialogs: HashMap<String, (String, String)> = HashMap::new();
    
    while let Some(event) = events.recv().await {
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
                
                // Get the Call-ID for dialog tracking
                let call_id = request.call_id()
                    .map(|header| header.value().to_string())
                    .unwrap_or_default();
                
                match request.method() {
                    Method::Invite => {
                        process_invite(server_tm.clone(), server_tx, request, source, &mut active_dialogs).await;
                    },
                    Method::Bye => {
                        process_bye(server_tm.clone(), server_tx, request, &mut active_dialogs).await;
                    },
                    _ => {
                        // For other methods, just send a 200 OK
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
            TransactionEvent::StateChanged { transaction_id, previous_state, new_state } => {
                debug!("Server transaction {} changed state: {:?} -> {:?}",
                    transaction_id, previous_state, new_state);
            },
            TransactionEvent::TransportError { transaction_id, .. } => {
                error!("Server transport error for transaction {}", transaction_id);
            },
            _ => {}
        }
    }
}

async fn process_invite(
    server_tm: TransactionManager,
    transaction_id: TransactionKey,
    request: Request,
    source: SocketAddr,
    active_dialogs: &mut HashMap<String, (String, String)>,
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
    
    // Wait a bit more to simulate phone ringing
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Get the From tag for dialog tracking
    let from_tag = request.from()
        .and_then(|from| from.tag())
        .unwrap_or_default();
    
    // Generate a local tag for the To header
    let to_tag = format!("tag-{}", Uuid::new_v4().simple());
    
    // Create a 200 OK response to accept the call
    let mut ok_builder = SimpleResponseBuilder::response_from_request(
        &request,
        StatusCode::Ok,
        Some("OK"),
    );
    
    // Add To tag to create dialog
    if let Some(to_header) = request.to() {
        let display_name = to_header.address().display_name().map(|s| s.to_string());
        let uri = to_header.address().uri.to_string();
        
        if let Some(display) = display_name {
            ok_builder = ok_builder.to(&display, &uri, Some(&to_tag));
        } else {
            ok_builder = ok_builder.to("", &uri, Some(&to_tag));
        }
    }
    
    let ok = ok_builder.build();
    
    // Store the dialog information
    if let Some(call_id) = request.call_id().map(|h| h.value().to_string()) {
        active_dialogs.insert(call_id, (from_tag.to_string(), to_tag));
    }
    
    if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
        error!("Failed to send OK response: {}", e);
    }
}

async fn process_bye(
    server_tm: TransactionManager,
    transaction_id: TransactionKey,
    request: Request,
    active_dialogs: &mut HashMap<String, (String, String)>,
) {
    // Check if this BYE is part of an active dialog
    let call_id = request.call_id()
        .map(|header| header.value().to_string())
        .unwrap_or_default();
    
    // Create a 200 OK response for the BYE
    let ok = SimpleResponseBuilder::response_from_request(
        &request,
        StatusCode::Ok,
        Some("OK"),
    ).build();
    
    if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
        error!("Failed to send OK response to BYE: {}", e);
    }
    
    // Remove the dialog
    active_dialogs.remove(&call_id);
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
                        info!("Received final success response: {} {}",
                            response.status_code(), response.reason_phrase());
                        return Ok(response);
                    },
                    TransactionEvent::FailureResponse { transaction_id: tx_id, response }
                        if tx_id == *transaction_id => {
                        info!("Received final failure response: {} {}",
                            response.status_code(), response.reason_phrase());
                        return Ok(response);
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

// Import for HashMap
use std::collections::HashMap; 