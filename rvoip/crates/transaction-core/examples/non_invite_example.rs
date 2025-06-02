/**
 * Non-INVITE Transaction Example
 * 
 * This example demonstrates non-INVITE transaction flows between
 * a SIP client and server using the **correct production APIs**. It shows:
 *
 * 1. Client sending an OPTIONS request (commonly used for keepalive)
 * 2. Server responding with 200 OK using automatic state machine
 * 3. Client sending a MESSAGE request (for instant messaging)
 * 4. Server responding with 200 OK using automatic state machine
 *
 * Unlike INVITE transactions, non-INVITE transactions:
 * - Don't require ACK for final responses
 * - Follow a simpler state machine (Trying → Proceeding → Completed → Terminated)
 * - Are single request-response exchanges
 * - Automatically terminate via RFC 3261 Timer K/J
 *
 * The example showcases **correct production usage patterns**:
 * - Using TransactionManager::subscribe_to_transaction() for event handling
 * - Handling TransactionEvent::StateChanged for state monitoring
 * - Using TransactionEvent::ProvisionalResponse, SuccessResponse for responses
 * - Leveraging automatic RFC 3261 compliant timers
 * - No manual timing or orchestration - pure event-driven architecture
 *
 * To run with full logging:
 * ```
 * RUST_LOG=rvoip=trace cargo run --example non_invite_example
 * ```
 */

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::Method;
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionState, TransactionKey};
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
use rvoip_transaction_core::builders::{client_quick, server_quick};

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup logging
    tracing_subscriber::fmt()
        .with_env_filter("rvoip=debug")
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
    
    // ------------- Main logic using correct production APIs -----------------
    
    // Spawn a task to handle server events
    tokio::spawn(handle_server_events(server_tm.clone(), server_events));
    
    // ------------- EXAMPLE 1: OPTIONS Request -----------------
    info!("EXAMPLE 1: OPTIONS Request using production APIs");
    
    // Create an OPTIONS request using the new builder
    let target_uri = format!("sip:server@{}", server_addr.ip());
    let from_uri = format!("sip:client@{}", client_addr.ip());
    
    let options_request = client_quick::options(&target_uri, &from_uri, client_addr)
        .expect("Failed to create OPTIONS request");
    
    // Create a client transaction for the OPTIONS request
    let options_tx_id = client_tm.create_client_transaction(options_request, server_addr).await?;
    info!("Created OPTIONS client transaction with ID: {}", options_tx_id);
    
    // Subscribe to this specific transaction's events using PRODUCTION API
    let mut options_events = client_tm.subscribe_to_transaction(&options_tx_id).await?;
    
    // Send the OPTIONS request - triggers automatic state machine
    client_tm.send_request(&options_tx_id).await?;
    info!("Sent OPTIONS request to server");
    
    // Handle OPTIONS events using proper event-driven pattern
    let mut options_completed = false;
    let timeout_duration = Duration::from_secs(3);
    let start_time = std::time::Instant::now();
    
    while !options_completed && start_time.elapsed() < timeout_duration {
        tokio::select! {
            Some(event) = options_events.recv() => {
                match event {
                    TransactionEvent::StateChanged { transaction_id, previous_state, new_state } 
                        if transaction_id == options_tx_id => {
                        info!("✅ OPTIONS transaction state: {:?} → {:?}", previous_state, new_state);
                        
                        if new_state == TransactionState::Completed || new_state == TransactionState::Terminated {
                            options_completed = true;
                        }
                    },
                    TransactionEvent::SuccessResponse { transaction_id, response, .. }
                        if transaction_id == options_tx_id => {
                        info!("✅ OPTIONS received final response: {} {}", 
                              response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::FailureResponse { transaction_id, response }
                        if transaction_id == options_tx_id => {
                        info!("✅ OPTIONS received failure response: {} {}", 
                              response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::TransactionTerminated { transaction_id }
                        if transaction_id == options_tx_id => {
                        info!("✅ OPTIONS transaction terminated via RFC 3261 timers");
                        options_completed = true;
                    },
                    _ => {}
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    
    // Give a short pause between requests
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // ------------- EXAMPLE 2: MESSAGE Request -----------------
    info!("EXAMPLE 2: MESSAGE Request using production APIs");
    
    // Create a MESSAGE request with text content using the new builder
    let message_content = "Hello, this is a SIP MESSAGE for instant messaging!";
    
    let message_request = client_quick::message(&target_uri, &from_uri, client_addr, message_content)
        .expect("Failed to create MESSAGE request");
    
    // Create a client transaction for the MESSAGE request
    let message_tx_id = client_tm.create_client_transaction(message_request, server_addr).await?;
    info!("Created MESSAGE client transaction with ID: {}", message_tx_id);
    
    // Subscribe to this specific transaction's events using PRODUCTION API
    let mut message_events = client_tm.subscribe_to_transaction(&message_tx_id).await?;
    
    // Send the MESSAGE request - triggers automatic state machine
    client_tm.send_request(&message_tx_id).await?;
    info!("Sent MESSAGE request to server");
    
    // Handle MESSAGE events using proper event-driven pattern
    let mut message_completed = false;
    let start_time = std::time::Instant::now();
    
    while !message_completed && start_time.elapsed() < timeout_duration {
        tokio::select! {
            Some(event) = message_events.recv() => {
                match event {
                    TransactionEvent::StateChanged { transaction_id, previous_state, new_state } 
                        if transaction_id == message_tx_id => {
                        info!("✅ MESSAGE transaction state: {:?} → {:?}", previous_state, new_state);
                        
                        if new_state == TransactionState::Completed || new_state == TransactionState::Terminated {
                            message_completed = true;
                        }
                    },
                    TransactionEvent::SuccessResponse { transaction_id, response, .. }
                        if transaction_id == message_tx_id => {
                        info!("✅ MESSAGE received final response: {} {}", 
                              response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::FailureResponse { transaction_id, response }
                        if transaction_id == message_tx_id => {
                        info!("✅ MESSAGE received failure response: {} {}", 
                              response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::TransactionTerminated { transaction_id }
                        if transaction_id == message_tx_id => {
                        info!("✅ MESSAGE transaction terminated via RFC 3261 timers");
                        message_completed = true;
                    },
                    _ => {}
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    
    if options_completed && message_completed {
        info!("✅ All non-INVITE transactions completed successfully using production APIs!");
    } else {
        warn!("⚠️  Test incomplete but demonstrates correct API usage - options: {}, message: {}", 
              options_completed, message_completed);
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
                info!("🔹 Server received {} request from {}", request.method(), source);
                
                match request.method() {
                    Method::Options => {
                        // Send 200 OK with Allow header listing supported methods
                        let ok = server_quick::ok_options(
                            &request, 
                            vec![Method::Invite, Method::Options, Method::Register, Method::Bye, Method::Cancel]
                        ).expect("Failed to create OPTIONS response");
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
                            error!("Failed to send OPTIONS response: {}", e);
                        } else {
                            info!("✅ Server sent 200 OK to OPTIONS");
                        }
                    },
                    Method::Message => {
                        // Send 200 OK for MESSAGE (instant messaging)
                        let ok = server_quick::ok_message(&request)
                            .expect("Failed to create MESSAGE response");
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
                            error!("Failed to send MESSAGE response: {}", e);
                        } else {
                            info!("✅ Server sent 200 OK to MESSAGE");
                        }
                    },
                    Method::Register => {
                        // Send 200 OK with registration info
                        let ok = server_quick::ok_register(
                            &request, 
                            3600, 
                            vec![format!("sip:user@{}", source.ip())]
                        ).expect("Failed to create REGISTER response");
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
                            error!("Failed to send REGISTER response: {}", e);
                        } else {
                            info!("✅ Server sent 200 OK to REGISTER");
                        }
                    },
                    _ => {
                        warn!("🤷 Server received unexpected {} request", request.method());
                        
                        // Send 501 Not Implemented for unsupported methods
                        let not_implemented = server_quick::server_error(&request, Some("Not Implemented".to_string()))
                            .expect("Failed to create 501 response");
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, not_implemented).await {
                            error!("Failed to send 501 response: {}", e);
                        }
                    }
                }
            },
            TransactionEvent::StateChanged { transaction_id, previous_state, new_state } => {
                debug!("🔹 Server transaction {} changed state: {:?} -> {:?}",
                    transaction_id, previous_state, new_state);
            },
            TransactionEvent::TransportError { transaction_id, .. } => {
                error!("🔹 Server transport error for transaction {}", transaction_id);
            },
            other_event => {
                debug!("🔄 Server received other event: {:?}", other_event);
            }
        }
    }
    
    info!("🛑 Server event handler shutting down");
} 