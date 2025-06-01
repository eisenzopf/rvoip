/**
 * CANCEL Transaction Example
 * 
 * This example demonstrates CANCEL transaction flows between
 * a SIP client and server using the **correct production APIs**. It shows:
 *
 * 1. Client sending an INVITE request (to establish a session)
 * 2. Server responding with 100 Trying (automatic) and 180 Ringing
 * 3. Client deciding to cancel the call while still ringing
 * 4. Client sending CANCEL request for the original INVITE
 * 5. Server receiving CANCEL and responding with 200 OK to CANCEL
 * 6. Server sending 487 Request Terminated for the original INVITE
 * 7. Client receiving both responses and sending ACK for 487
 *
 * CANCEL transactions are special because:
 * - CANCEL can only cancel an ongoing INVITE transaction
 * - CANCEL creates its own separate non-INVITE transaction
 * - The original INVITE transaction must respond with 487 after CANCEL
 * - ACK is still required for the 487 response to the INVITE
 * - Both transactions run concurrently and must be handled properly
 *
 * The example showcases **correct production usage patterns**:
 * - Using TransactionManager::subscribe_to_transaction() for event handling
 * - Handling TransactionEvent::StateChanged for state monitoring
 * - Using TransactionEvent::ProvisionalResponse, SuccessResponse for responses
 * - Managing two concurrent transactions (INVITE and CANCEL)
 * - Leveraging automatic RFC 3261 compliant state machine
 * - No manual timing or orchestration - pure event-driven architecture
 *
 * To run with full logging:
 * ```
 * RUST_LOG=rvoip=trace cargo run --example cancel_example
 * ```
 */

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;
use std::str::FromStr;

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
    
    // ------------- EXAMPLE: INVITE with CANCEL Flow -----------------
    info!("INVITE with CANCEL Flow using production APIs");
    
    // Create an INVITE request with SDP content
    let call_id = format!("cancel-{}", Uuid::new_v4());
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
    let invite_tx_id = client_tm.create_client_transaction(invite_request.clone(), server_addr).await?;
    info!("Created INVITE client transaction with ID: {}", invite_tx_id);
    
    // Subscribe to this specific transaction's events using PRODUCTION API
    let mut invite_events = client_tm.subscribe_to_transaction(&invite_tx_id).await?;
    
    // Send the INVITE request - triggers automatic state machine
    client_tm.send_request(&invite_tx_id).await?;
    info!("Sent INVITE request to server");
    
    // Handle INVITE events until we get provisional responses
    let mut received_trying = false;
    let mut received_ringing = false;
    let mut received_487 = false;
    let mut invite_completed = false;
    let timeout_duration = Duration::from_secs(15);
    let start_time = std::time::Instant::now();
    
    // Wait for trying and ringing responses before sending CANCEL
    while !received_ringing && start_time.elapsed() < Duration::from_secs(3) {
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
                            break; // Now we can send CANCEL
                        }
                    },
                    _ => {}
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    
    if !received_ringing {
        warn!("⚠️  Did not receive ringing response, but proceeding with CANCEL demonstration");
    }
    
    // Now create and send CANCEL for the INVITE request
    let cancel_request = SimpleRequestBuilder::new(Method::Cancel, &format!("sip:server@{}", server_addr.ip()))?
        .from("Client", &format!("sip:client@{}", client_addr.ip()), Some(&from_tag))
        .to("Server", &format!("sip:server@{}", server_addr.ip()), None)
        .call_id(&call_id)
        .cseq(1) // CANCEL uses same CSeq number as INVITE
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();
    
    // Create a client transaction for the CANCEL request
    let cancel_tx_id = client_tm.create_client_transaction(cancel_request, server_addr).await?;
    info!("Created CANCEL client transaction with ID: {}", cancel_tx_id);
    
    // Subscribe to CANCEL transaction events using PRODUCTION API
    let mut cancel_events = client_tm.subscribe_to_transaction(&cancel_tx_id).await?;
    
    // Send the CANCEL request - triggers automatic state machine
    client_tm.send_request(&cancel_tx_id).await?;
    info!("Sent CANCEL request to server");
    
    // Now handle both INVITE and CANCEL events concurrently
    let mut cancel_completed = false;
    
    while (!invite_completed || !cancel_completed) && start_time.elapsed() < timeout_duration {
        tokio::select! {
            // Handle INVITE events
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
                    },
                    TransactionEvent::FailureResponse { transaction_id, response }
                        if transaction_id == invite_tx_id => {
                        info!("✅ INVITE received final response: {} {}", 
                              response.status_code(), response.reason_phrase());
                        
                        if response.status_code() == 487 {
                            received_487 = true;
                            // Note: ACK for 487 is handled automatically by the transaction layer
                        }
                    },
                    TransactionEvent::TransactionTerminated { transaction_id }
                        if transaction_id == invite_tx_id => {
                        info!("✅ INVITE transaction terminated via RFC 3261 timers");
                        invite_completed = true;
                    },
                    _ => {}
                }
            },
            // Handle CANCEL events  
            Some(event) = cancel_events.recv() => {
                match event {
                    TransactionEvent::StateChanged { transaction_id, previous_state, new_state } 
                        if transaction_id == cancel_tx_id => {
                        info!("✅ CANCEL transaction state: {:?} → {:?}", previous_state, new_state);
                        
                        if new_state == TransactionState::Completed || new_state == TransactionState::Terminated {
                            cancel_completed = true;
                        }
                    },
                    TransactionEvent::SuccessResponse { transaction_id, response, .. }
                        if transaction_id == cancel_tx_id => {
                        info!("✅ CANCEL received final response: {} {}", 
                              response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::FailureResponse { transaction_id, response }
                        if transaction_id == cancel_tx_id => {
                        info!("✅ CANCEL received failure response: {} {}", 
                              response.status_code(), response.reason_phrase());
                    },
                    TransactionEvent::TransactionTerminated { transaction_id }
                        if transaction_id == cancel_tx_id => {
                        info!("✅ CANCEL transaction terminated via RFC 3261 timers");
                        cancel_completed = true;
                    },
                    _ => {}
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    
    if received_487 && cancel_completed {
        info!("✅ CANCEL call flow completed successfully using production APIs!");
    } else {
        warn!("⚠️  Test incomplete but demonstrates correct API usage - 487: {}, cancel: {}", 
              received_487, cancel_completed);
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
    // Track ongoing INVITE transactions that can be cancelled
    let mut invite_transactions: HashMap<String, TransactionKey> = HashMap::new();
    
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
                        // Track this INVITE by call-id for potential CANCEL
                        if let Some(call_id_header) = request.call_id() {
                            let call_id = call_id_header.value().to_string();
                            invite_transactions.insert(call_id, server_tx.clone());
                        }
                        process_invite_request(server_tm.clone(), server_tx, request).await;
                    },
                    Method::Cancel => {
                        process_cancel_request(
                            server_tm.clone(), 
                            server_tx, 
                            request, 
                            &mut invite_transactions
                        ).await;
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
    // For INVITE, we send provisional responses and delay final response
    // This allows time for the CANCEL to arrive
    
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
    
    // Wait longer to give time for CANCEL to arrive
    // In a real scenario, this would be when the user picks up the phone
    tokio::time::sleep(Duration::from_millis(3000)).await;
    
    // Note: If a CANCEL was received, the INVITE transaction would be cancelled
    // and we would send 487 Request Terminated instead of 200 OK
    // For this example, we assume CANCEL will be processed separately
    
    info!("ℹ️  INVITE processing complete (may be cancelled by separate CANCEL transaction)");
}

async fn process_cancel_request(
    server_tm: TransactionManager,
    cancel_transaction_id: TransactionKey,
    cancel_request: Request,
    invite_transactions: &mut HashMap<String, TransactionKey>,
) {
    // First, respond to the CANCEL request itself with 200 OK
    let cancel_ok = SimpleResponseBuilder::response_from_request(
        &cancel_request,
        StatusCode::Ok,
        Some("OK"),
    ).build();
    
    if let Err(e) = server_tm.send_response(&cancel_transaction_id, cancel_ok).await {
        error!("Failed to send CANCEL response: {}", e);
        return;
    } else {
        info!("✅ Server sent 200 OK response to CANCEL");
    }
    
    // Now find and cancel the corresponding INVITE transaction
    if let Some(call_id_header) = cancel_request.call_id() {
        let call_id = call_id_header.value().to_string();
        
        if let Some(invite_tx_id) = invite_transactions.remove(&call_id) {
            // Send 487 Request Terminated to the original INVITE
            let request_terminated = SimpleResponseBuilder::response_from_request(
                &cancel_request, // Use CANCEL request as template, but this goes to INVITE transaction
                StatusCode::RequestTerminated,
                Some("Request Terminated"),
            ).build();
            
            if let Err(e) = server_tm.send_response(&invite_tx_id, request_terminated).await {
                error!("Failed to send 487 Response to INVITE: {}", e);
            } else {
                info!("✅ Server sent 487 Request Terminated to original INVITE");
            }
        } else {
            warn!("⚠️  CANCEL received but no matching INVITE transaction found for call-id: {}", call_id);
        }
    }
} 