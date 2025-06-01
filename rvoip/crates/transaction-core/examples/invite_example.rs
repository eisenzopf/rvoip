/**
 * INVITE Transaction Example
 * 
 * This example demonstrates INVITE transaction flows between
 * a SIP client and server using the **correct production APIs**. It shows:
 *
 * 1. Client sending an INVITE request (to establish a session)
 * 2. Server responding with 100 Trying (automatic) and 180 Ringing
 * 3. Client receiving and processing provisional responses
 * 4. Server sending 200 OK final response with SDP
 * 5. Client receiving 200 OK and sending ACK
 * 6. Session termination with BYE request/response exchange
 *
 * Unlike non-INVITE transactions, INVITE transactions:
 * - Require ACK for 2xx final responses (handled automatically)
 * - Use a more complex state machine (Initial → Calling → Proceeding → Completed → Terminated)
 * - Can receive multiple provisional responses
 * - Use different timers (Timer A/B for retransmissions, Timer D for cleanup)
 *
 * The example showcases **correct production usage patterns**:
 * - Using TransactionManager::subscribe_to_transaction() for event handling
 * - Handling TransactionEvent::StateChanged for state monitoring
 * - Using TransactionEvent::ProvisionalResponse, SuccessResponse for responses
 * - Leveraging automatic RFC 3261 compliant state machine
 * - No manual timing or orchestration - pure event-driven architecture
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
    
    // ------------- Main logic using correct production APIs -----------------
    
    // Spawn a task to handle server events
    tokio::spawn(handle_server_events(server_tm.clone(), server_events));
    
    // ------------- EXAMPLE: INVITE Call Flow -----------------
    info!("INVITE Call Flow using production APIs");
    
    // Create an INVITE request with SDP content
    let call_id = format!("invite-{}", Uuid::new_v4());
    let from_tag = format!("tag-{}", Uuid::new_v4().simple());
    let sdp_content = r#"v=0
o=client 123456 123456 IN IP4 127.0.0.1
s=Session
c=IN IP4 127.0.0.1
t=0 0
m=audio 5004 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#;
    
    let invite_request = SimpleRequestBuilder::new(Method::Invite, &format!("sip:server@{}", server_addr.ip()))?
        .from("Client", &format!("sip:client@{}", client_addr.ip()), Some(&from_tag))
        .to("Server", &format!("sip:server@{}", server_addr.ip()), None)
        .call_id(&call_id)
        .cseq(1)
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentType(ContentType::from_str("application/sdp").unwrap()))
        .header(TypedHeader::ContentLength(ContentLength::new(sdp_content.len() as u32)))
        .body(sdp_content.as_bytes().to_vec())
        .build();
    
    // Create a client transaction for the INVITE request
    let invite_tx_id = client_tm.create_client_transaction(invite_request, server_addr).await?;
    info!("Created INVITE client transaction with ID: {}", invite_tx_id);
    
    // Subscribe to this specific transaction's events using PRODUCTION API
    let mut invite_events = client_tm.subscribe_to_transaction(&invite_tx_id).await?;
    
    // Send the INVITE request - triggers automatic state machine
    client_tm.send_request(&invite_tx_id).await?;
    info!("Sent INVITE request to server");
    
    // Handle INVITE events using proper event-driven pattern
    let mut received_trying = false;
    let mut received_ringing = false;
    let mut received_200_ok = false;
    let mut invite_completed = false;
    let timeout_duration = Duration::from_secs(10);
    let start_time = std::time::Instant::now();
    
    while !invite_completed && start_time.elapsed() < timeout_duration {
        tokio::select! {
            Some(event) = invite_events.recv() => {
                match event {
                    TransactionEvent::StateChanged { transaction_id, previous_state, new_state } 
                        if transaction_id == invite_tx_id => {
                        info!("✅ INVITE transaction state: {:?} → {:?}", previous_state, new_state);
                        
                        if new_state == TransactionState::Completed || new_state == TransactionState::Terminated {
                            invite_completed = true;
                        }
                    },
                    TransactionEvent::ProvisionalResponse { transaction_id, response } 
                        if transaction_id == invite_tx_id => {
                        let status = response.status_code();
                        info!("✅ INVITE received provisional response: {} {}", 
                              status, response.reason_phrase());
                        
                        if status == 100 {
                            received_trying = true;
                        } else if status == 180 {
                            received_ringing = true;
                        }
                    },
                    TransactionEvent::SuccessResponse { transaction_id, response, .. }
                        if transaction_id == invite_tx_id => {
                        info!("✅ INVITE received final response: {} {}", 
                              response.status_code(), response.reason_phrase());
                        received_200_ok = true;
                        // Note: ACK is handled automatically by the transaction layer
                    },
                    TransactionEvent::FailureResponse { transaction_id, response }
                        if transaction_id == invite_tx_id => {
                        info!("✅ INVITE received failure response: {} {}", 
                              response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::TransactionTerminated { transaction_id }
                        if transaction_id == invite_tx_id => {
                        info!("✅ INVITE transaction terminated via RFC 3261 timers");
                        invite_completed = true;
                    },
                    _ => {}
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    
    // Give some time for the session to be established
    tokio::time::sleep(Duration::from_millis(1000)).await;
    
    // ------------- Session Termination with BYE -----------------
    info!("Session termination with BYE using production APIs");
    
    // Create a BYE request to terminate the session
    let bye_request = SimpleRequestBuilder::new(Method::Bye, &format!("sip:server@{}", server_addr.ip()))?
        .from("Client", &format!("sip:client@{}", client_addr.ip()), Some(&from_tag))
        .to("Server", &format!("sip:server@{}", server_addr.ip()), None)
        .call_id(&call_id)
        .cseq(2) // Increment CSeq for new request in same dialog
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();
    
    // Create a client transaction for the BYE request
    let bye_tx_id = client_tm.create_client_transaction(bye_request, server_addr).await?;
    info!("Created BYE client transaction with ID: {}", bye_tx_id);
    
    // Subscribe to this specific transaction's events using PRODUCTION API
    let mut bye_events = client_tm.subscribe_to_transaction(&bye_tx_id).await?;
    
    // Send the BYE request - triggers automatic state machine
    client_tm.send_request(&bye_tx_id).await?;
    info!("Sent BYE request to server");
    
    // Handle BYE events using proper event-driven pattern
    let mut bye_completed = false;
    let start_time = std::time::Instant::now();
    
    while !bye_completed && start_time.elapsed() < timeout_duration {
        tokio::select! {
            Some(event) = bye_events.recv() => {
                match event {
                    TransactionEvent::StateChanged { transaction_id, previous_state, new_state } 
                        if transaction_id == bye_tx_id => {
                        info!("✅ BYE transaction state: {:?} → {:?}", previous_state, new_state);
                        
                        if new_state == TransactionState::Completed || new_state == TransactionState::Terminated {
                            bye_completed = true;
                        }
                    },
                    TransactionEvent::SuccessResponse { transaction_id, response, .. }
                        if transaction_id == bye_tx_id => {
                        info!("✅ BYE received final response: {} {}", 
                              response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::FailureResponse { transaction_id, response }
                        if transaction_id == bye_tx_id => {
                        info!("✅ BYE received failure response: {} {}", 
                              response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::TransactionTerminated { transaction_id }
                        if transaction_id == bye_tx_id => {
                        info!("✅ BYE transaction terminated via RFC 3261 timers");
                        bye_completed = true;
                    },
                    _ => {}
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    
    if received_200_ok && bye_completed {
        info!("✅ Complete INVITE call flow completed successfully using production APIs!");
    } else {
        warn!("⚠️  Test incomplete but demonstrates correct API usage - invite: {}, bye: {}", 
              received_200_ok, bye_completed);
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
                
                // Create a server transaction using proper API
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
                
                // Process based on request method using automatic state machine
                match request.method() {
                    Method::Invite => {
                        process_invite_request(server_tm.clone(), server_tx, request).await;
                    },
                    Method::Bye => {
                        process_bye_request(server_tm.clone(), server_tx, request).await;
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
                        } else {
                            info!("✅ Server sent 200 OK response");
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

async fn process_invite_request(
    server_tm: TransactionManager,
    transaction_id: TransactionKey,
    request: Request,
) {
    // For INVITE, we send provisional responses and then a final response
    
    // The 100 Trying is sent automatically by the transaction layer
    
    // Wait a bit to simulate processing, then send 180 Ringing
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    let ringing = SimpleResponseBuilder::response_from_request(
        &request,
        StatusCode::Ringing,
        Some("Ringing"),
    ).build();
    
    if let Err(e) = server_tm.send_response(&transaction_id, ringing).await {
        error!("Failed to send Ringing response: {}", e);
    } else {
        info!("✅ Server sent 180 Ringing response");
    }
    
    // Wait a bit more to simulate user answering, then send 200 OK
    tokio::time::sleep(Duration::from_millis(1000)).await;
    
    // Create a 200 OK response with SDP answer
    let sdp_answer = r#"v=0
o=server 654321 654321 IN IP4 127.0.0.1
s=Session
c=IN IP4 127.0.0.1
t=0 0
m=audio 5006 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#;
    
    let ok = SimpleResponseBuilder::response_from_request(
        &request,
        StatusCode::Ok,
        Some("OK"),
    )
    .header(TypedHeader::ContentType(ContentType::from_str("application/sdp").unwrap()))
    .header(TypedHeader::ContentLength(ContentLength::new(sdp_answer.len() as u32)))
    .body(sdp_answer.as_bytes().to_vec())
    .build();
    
    if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
        error!("Failed to send OK response: {}", e);
    } else {
        info!("✅ Server sent 200 OK response with SDP answer");
    }
}

async fn process_bye_request(
    server_tm: TransactionManager,
    transaction_id: TransactionKey,
    request: Request,
) {
    // For BYE, just send 200 OK immediately
    let ok = SimpleResponseBuilder::response_from_request(
        &request,
        StatusCode::Ok,
        Some("OK"),
    ).build();
    
    if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
        error!("Failed to send BYE response: {}", e);
    } else {
        info!("✅ Server sent 200 OK response to BYE");
    }
}

// Import for parsing content type
use std::str::FromStr; 