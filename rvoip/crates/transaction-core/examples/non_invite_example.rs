/**
 * Non-INVITE Transaction Example
 * 
 * This example demonstrates non-INVITE transaction flows between
 * a SIP client and server using the transaction-core and sip-transport integration.
 * The example shows:
 *
 * 1. Client sending an OPTIONS request (commonly used for keepalive)
 * 2. Server responding with 200 OK
 * 3. Client sending a MESSAGE request (for instant messaging)
 * 4. Server responding with 200 OK
 *
 * Unlike INVITE transactions, non-INVITE transactions:
 * - Don't require ACK for final responses
 * - Follow a simpler state machine (Trying → Proceeding → Completed → Terminated)
 * - Are single request-response exchanges
 *
 * To run with full logging:
 * ```
 * RUST_LOG=rvoip=trace cargo run --example non_invite_example
 * ```
 */

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip_core::{Method, Message, Request, Response, Uri};
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::status::StatusCode;
use rvoip_sip_core::types::header::{HeaderName, TypedHeader};
use rvoip_sip_core::types::content_type::ContentType;
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
    
    // ------------- Main logic -----------------
    
    // Spawn a task to handle server events
    tokio::spawn(handle_server_events(server_tm.clone(), server_events));
    
    // ------------- EXAMPLE 1: OPTIONS Request -----------------
    info!("EXAMPLE 1: OPTIONS Request");
    
    // Create an OPTIONS request
    let call_id1 = format!("options-{}", Uuid::new_v4());
    let from_tag1 = format!("tag-{}", Uuid::new_v4().simple());
    
    let options_request = SimpleRequestBuilder::new(Method::Options, &format!("sip:server@{}", server_addr.ip()))?
        .from("Client", &format!("sip:client@{}", client_addr.ip()), Some(&from_tag1))
        .to("Server", &format!("sip:server@{}", server_addr.ip()), None)
        .call_id(&call_id1)
        .cseq(1)
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();
    
    // Create a client transaction for the OPTIONS request
    let options_tx_id = client_tm.create_client_transaction(options_request, server_addr).await?;
    info!("Created OPTIONS client transaction with ID: {}", options_tx_id);
    
    // Send the OPTIONS request
    client_tm.send_request(&options_tx_id).await?;
    info!("Sent OPTIONS request to server");
    
    // Wait for response
    let options_response = wait_for_final_response(&mut client_events, &options_tx_id).await?;
    info!("Received {} response for OPTIONS: {}", 
            options_response.status_code(), 
            options_response.reason_phrase());
            
    // Give a short pause between requests
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // ------------- EXAMPLE 2: MESSAGE Request -----------------
    info!("EXAMPLE 2: MESSAGE Request");
    
    // Create a MESSAGE request with text content
    let call_id2 = format!("message-{}", Uuid::new_v4());
    let from_tag2 = format!("tag-{}", Uuid::new_v4().simple());
    let message_content = "Hello, this is a SIP MESSAGE for instant messaging!";
    
    let message_request = SimpleRequestBuilder::new(Method::Message, &format!("sip:server@{}", server_addr.ip()))?
        .from("Client", &format!("sip:client@{}", client_addr.ip()), Some(&from_tag2))
        .to("Server", &format!("sip:server@{}", server_addr.ip()), None)
        .call_id(&call_id2)
        .cseq(1)
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentType(ContentType::from_str("text/plain").unwrap()))
        .header(TypedHeader::ContentLength(ContentLength::new(message_content.len() as u32)))
        .body(message_content.as_bytes().to_vec())
        .build();
    
    // Create a client transaction for the MESSAGE request
    let message_tx_id = client_tm.create_client_transaction(message_request, server_addr).await?;
    info!("Created MESSAGE client transaction with ID: {}", message_tx_id);
    
    // Send the MESSAGE request
    client_tm.send_request(&message_tx_id).await?;
    info!("Sent MESSAGE request to server");
    
    // Wait for response
    let message_response = wait_for_final_response(&mut client_events, &message_tx_id).await?;
    info!("Received {} response for MESSAGE: {}", 
            message_response.status_code(), 
            message_response.reason_phrase());
    
    // All transactions completed successfully
    info!("All non-INVITE transactions completed successfully");
    
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
                
                // Process based on request method
                match request.method() {
                    Method::Options => {
                        process_options_request(server_tm.clone(), server_tx, request).await;
                    },
                    Method::Message => {
                        process_message_request(server_tm.clone(), server_tx, request).await;
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

async fn process_options_request(
    server_tm: TransactionManager,
    transaction_id: TransactionKey,
    request: Request,
) {
    // For OPTIONS, we respond with 200 OK and include supported methods
    let mut ok_builder = SimpleResponseBuilder::response_from_request(
        &request,
        StatusCode::Ok,
        Some("OK"),
    );
    
    // Add the Allow header to indicate supported methods
    ok_builder = ok_builder.header(TypedHeader::Allow(
        "INVITE, ACK, CANCEL, OPTIONS, BYE, REGISTER, MESSAGE".parse().unwrap()
    ));
    
    let ok = ok_builder.build();
    
    if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
        error!("Failed to send OPTIONS response: {}", e);
    }
}

async fn process_message_request(
    server_tm: TransactionManager,
    transaction_id: TransactionKey,
    request: Request,
) {
    // Extract the message content
    let body = request.body();
    if !body.is_empty() {
        if let Ok(message_text) = std::str::from_utf8(body) {
            info!("Received instant message: {}", message_text);
        }
    }
    
    // Send 200 OK response
    let ok = SimpleResponseBuilder::response_from_request(
        &request,
        StatusCode::Ok,
        Some("OK"),
    ).build();
    
    if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
        error!("Failed to send MESSAGE response: {}", e);
    }
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

// Import for parsing content type
use std::str::FromStr; 