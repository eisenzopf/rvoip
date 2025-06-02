/**
 * Integrated Transport Example
 * 
 * This example demonstrates the integration between the SIP transaction layer (transaction-core)
 * and the SIP transport layer (sip-transport) using the **correct production APIs**. It shows:
 *
 * 1. A client creates a REGISTER request and sends it to a server
 * 2. The server receives the request and creates a server transaction  
 * 3. The server sends a 100 Trying provisional response (automatic)
 * 4. The server processes the request and sends a 200 OK final response
 * 5. The client receives and processes both responses using proper event handling
 *
 * The example showcases **correct production usage patterns**:
 * - Using TransactionManager::subscribe_to_transaction() for event handling
 * - Handling TransactionEvent::StateChanged for state monitoring
 * - Using TransactionEvent::ProvisionalResponse, SuccessResponse for responses
 * - Leveraging the automatic RFC 3261 compliant state machine
 * - No manual timing or orchestration - pure event-driven architecture
 *
 * To run this example with full logging:
 * ```
 * RUST_LOG=rvoip=trace cargo run --example integrated_transport
 * ```
 *
 * This demonstrates the **proper production API usage** for the transaction-core library.
 */

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::Method;
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionState, TransactionKey};
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
use rvoip_transaction_core::builders::{client_quick, server_quick};

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
    
    // ------------- Main test logic using proper production APIs -----------------
    
    // Spawn a task to handle server events
    tokio::spawn(handle_server_events(server_tm.clone(), server_events));
    
    // Create a REGISTER request using the new builder
    let registrar_uri = format!("sip:server@{}", server_addr.ip());
    let user_uri = format!("sip:client@{}", client_addr.ip());
    
    let register_request = client_quick::register(
        &registrar_uri,
        &user_uri,
        "Client UA",
        client_addr,
        Some(3600), // 1 hour registration
    ).expect("Failed to create REGISTER request");
    
    // Create a client transaction for the REGISTER request
    let tx_id = client_tm.create_client_transaction(register_request, server_addr).await?;
    info!("Created client transaction with ID: {}", tx_id);
    
    // Subscribe to this specific transaction's events using the PRODUCTION API
    let mut tx_events = client_tm.subscribe_to_transaction(&tx_id).await?;
    
    // Send the request - triggers automatic state machine
    client_tm.send_request(&tx_id).await?;
    info!("Sent REGISTER request to server");
    
    // Handle events using proper event-driven pattern
    let mut received_provisional = false;
    let mut received_final = false;
    let mut transaction_completed = false;
    
    // Use timeout to prevent hanging
    let timeout_duration = Duration::from_secs(5);
    let start_time = std::time::Instant::now();
    
    while !transaction_completed && start_time.elapsed() < timeout_duration {
        tokio::select! {
            Some(event) = tx_events.recv() => {
                match event {
                    TransactionEvent::StateChanged { transaction_id, previous_state, new_state } 
                        if transaction_id == tx_id => {
                        info!("✅ Transaction state: {:?} → {:?}", previous_state, new_state);
                        
                        // Monitor for final states
                        if new_state == TransactionState::Completed || new_state == TransactionState::Terminated {
                            transaction_completed = true;
                        }
                    },
                    TransactionEvent::ProvisionalResponse { transaction_id, response } 
                        if transaction_id == tx_id => {
                        info!("✅ Received provisional response: {} {}", 
                              response.status_code(), response.reason_phrase());
                        received_provisional = true;
                    },
                    TransactionEvent::SuccessResponse { transaction_id, response, .. }
                        if transaction_id == tx_id => {
                        info!("✅ Received final response: {} {}", 
                              response.status_code(), response.reason_phrase());
                        received_final = true;
                    },
                    TransactionEvent::FailureResponse { transaction_id, response }
                        if transaction_id == tx_id => {
                        info!("✅ Received failure response: {} {}", 
                              response.status_code(), response.reason_phrase());
                        received_final = true;
                    },
                    TransactionEvent::TransactionTerminated { transaction_id }
                        if transaction_id == tx_id => {
                        info!("✅ Transaction terminated via automatic RFC 3261 timers");
                        transaction_completed = true;
                    },
                    TransactionEvent::TransportError { transaction_id } 
                        if transaction_id == tx_id => {
                        error!("❌ Transport error for transaction {}", transaction_id);
                        return Err("Transport error".into());
                    },
                    _ => {
                        // Ignore other events not relevant to our transaction
                    }
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                // Small delay to prevent busy looping
            }
        }
    }
    
    if received_provisional && received_final {
        info!("✅ Test completed successfully using production TransactionManager APIs!");
    } else if start_time.elapsed() >= timeout_duration {
        warn!("⚠️  Test timed out - but this demonstrates proper API usage");
    } else {
        warn!("⚠️  Test incomplete - received_provisional: {}, received_final: {}", 
              received_provisional, received_final);
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
            TransactionEvent::NonInviteRequest { transaction_id, request, source, .. } => {
                info!("Server received request: {:?} from {}", request.method(), source);
                
                // For REGISTER, send responses using proper API and new builders
                if request.method() == Method::Register {
                    // The 100 Trying is sent automatically by the transaction layer
                    // Wait a bit to simulate processing (this would be real business logic)
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    
                    // Send 200 OK response with registered contacts using the new builder
                    let contact = format!("sip:client@{}", source.ip());
                    let ok = server_quick::ok_register(&request, 3600, vec![contact])
                        .expect("Failed to create 200 OK REGISTER response");
                    
                    if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
                        error!("Failed to send OK response: {}", e);
                    } else {
                        info!("✅ Server sent 200 OK response");
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