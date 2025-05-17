/**
 * Integrated Transport Example
 * 
 * This example demonstrates the integration between the SIP transaction layer (transaction-core)
 * and the SIP transport layer (sip-transport). It simulates a full SIP exchange where:
 *
 * 1. A client creates a REGISTER request and sends it to a server
 * 2. The server receives the request and creates a server transaction
 * 3. The server sends a 100 Trying provisional response
 * 4. The server processes the request and sends a 200 OK final response
 * 5. The client receives and processes both responses
 *
 * The example shows how the two layers work together:
 * - The transport layer handles the actual sending and receiving of SIP messages
 * - The transaction layer manages transaction state, retransmissions, and event notifications
 *
 * To run this example with full logging:
 * ```
 * RUST_LOG=rvoip=trace cargo run --example integrated_transport
 * ```
 *
 * This is a self-contained test that creates both client and server in a single process,
 * but the same principles apply when client and server are running on different machines.
 */

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip_core::{Method, Request, Response, Message};
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::types::status::StatusCode;

use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionState};
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use tracing_subscriber::fmt::format::FmtSpan;

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
    
    // ------------- Main test logic -----------------
    
    // Spawn a task to handle server events
    tokio::spawn(handle_server_events(server_tm.clone(), server_events));
    
    // Create a REGISTER request
    let register_request = SimpleRequestBuilder::new(Method::Register, &format!("sip:server@{}", server_addr.ip()))
        .unwrap()
        .from("client", &format!("sip:client@{}", client_addr.ip()), Some("tag1"))
        .to("server", &format!("sip:server@{}", server_addr.ip()), None)
        .call_id(&format!("call-{}", uuid::Uuid::new_v4()))
        .cseq(1)
        .contact(&format!("sip:client@{}", client_addr.ip()), None)
        .build();
    
    // Create a client transaction for the REGISTER request
    let tx_id = client_tm.create_client_transaction(register_request, server_addr).await?;
    info!("Created client transaction with ID: {}", tx_id);
    
    // Send the request
    client_tm.send_request(&tx_id).await?;
    info!("Sent REGISTER request to server");
    
    // Wait for events
    let mut timeout = false;
    let mut response_received = false;
    let timeout_duration = Duration::from_secs(5);
    let start_time = std::time::Instant::now();
    
    while !timeout && !response_received && start_time.elapsed() < timeout_duration {
        tokio::select! {
            Some(event) = client_events.recv() => {
                match event {
                    TransactionEvent::ProvisionalResponse { transaction_id, response, .. } 
                        if transaction_id == tx_id => {
                        info!("Received provisional response: {} {}",
                            response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::SuccessResponse { transaction_id, response, .. }
                        if transaction_id == tx_id => {
                        info!("Received final response: {} {}",
                            response.status_code(), response.reason_phrase());
                        
                        // Test completed successfully
                        response_received = true;
                    },
                    TransactionEvent::TransportError { transaction_id, .. } 
                        if transaction_id == tx_id => {
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
    
    if response_received {
        info!("Test completed successfully!");
    } else {
        warn!("Test timed out waiting for response");
    }
    
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
                
                // Create a server transaction
                let server_tx = match server_tm.create_server_transaction(
                    request.clone(),
                    source,
                ).await {
                    Ok(tx) => tx,
                    Err(e) => {
                        error!("Failed to create server transaction: {}", e);
                        continue;
                    }
                };
                
                // The transaction ID is now available from the server transaction
                let tx_id = server_tx.id().clone();
                
                // For REGISTER, send a 200 OK
                if request.method() == Method::Register {
                    // First send a 100 Trying
                    let trying = SimpleResponseBuilder::response_from_request(
                        &request,
                        StatusCode::Trying,
                        Some("Trying"),
                    ).build();
                    
                    if let Err(e) = server_tm.send_response(&tx_id, trying).await {
                        error!("Failed to send Trying response: {}", e);
                    }
                    
                    // Wait a bit to simulate processing
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    
                    // Then send a 200 OK
                    let ok = SimpleResponseBuilder::response_from_request(
                        &request,
                        StatusCode::Ok,
                        Some("OK"),
                    ).build();
                    
                    if let Err(e) = server_tm.send_response(&tx_id, ok).await {
                        error!("Failed to send OK response: {}", e);
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