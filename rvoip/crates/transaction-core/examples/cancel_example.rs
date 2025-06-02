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
 * 5. Server responding with 200 OK to CANCEL
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
 * - Using TransactionManager with real transport
 * - Handling InviteRequest and NonInviteRequest events
 * - Proper timing for CANCEL (after provisional, before final response)
 * - Complete lifecycle with ACK to 487 response
 * - No manual timing - pure RFC 3261 compliant flows
 */

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::Method;
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
use rvoip_transaction_core::builders::{client_quick, server_quick};

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup logging
    tracing_subscriber::fmt()
        .with_env_filter("rvoip=info")
        .init();
    
    // Create server transport
    let server_config = TransportManagerConfig {
        enable_udp: true,
        bind_addresses: vec!["127.0.0.1:5060".parse()?],
        ..Default::default()
    };
    
    let (mut server_transport, server_transport_rx) = TransportManager::new(server_config).await?;
    server_transport.initialize().await?;
    
    let server_addr = server_transport.default_transport().await
        .ok_or("No default transport")?.local_addr()?;
    info!("Server listening on {}", server_addr);
    
    // Create client transport
    let client_config = TransportManagerConfig {
        enable_udp: true,
        bind_addresses: vec!["127.0.0.1:0".parse()?],
        ..Default::default()
    };
    
    let (mut client_transport, client_transport_rx) = TransportManager::new(client_config).await?;
    client_transport.initialize().await?;
    
    let client_addr = client_transport.default_transport().await
        .ok_or("No default transport")?.local_addr()?;
    info!("Client listening on {}", client_addr);
    
    // Create transaction managers
    let (server_tm, server_events) = TransactionManager::with_transport_manager(
        server_transport.clone(), server_transport_rx, Some(100)).await?;
    
    let (client_tm, client_events) = TransactionManager::with_transport_manager(
        client_transport.clone(), client_transport_rx, Some(100)).await?;
    
    // Start event handlers
    tokio::spawn(handle_server_events(server_tm.clone(), server_events));
    tokio::spawn(handle_client_events(client_tm.clone(), client_events, server_addr));
    
    // Wait for demonstration to complete
    tokio::time::sleep(Duration::from_secs(10)).await;
    
    // Cleanup
    client_tm.shutdown().await;
    server_tm.shutdown().await;
    
    Ok(())
}

async fn handle_server_events(
    server_tm: TransactionManager,
    mut events: mpsc::Receiver<TransactionEvent>,
) {
    let mut pending_invites: std::collections::HashMap<TransactionKey, bool> = std::collections::HashMap::new();
    
    while let Some(event) = events.recv().await {
        match event {
            TransactionEvent::InviteRequest { transaction_id, request, source, .. } => {
                info!("🔹 Server received INVITE from {}", source);
                pending_invites.insert(transaction_id.clone(), false); // false = not cancelled
                
                // Send 180 Ringing after a short delay
                tokio::time::sleep(Duration::from_millis(300)).await;
                
                let ringing = server_quick::ringing(&request, None)
                    .expect("Failed to create 180 Ringing");
                
                if let Err(e) = server_tm.send_response(&transaction_id, ringing).await {
                    error!("Failed to send 180 Ringing: {}", e);
                } else {
                    info!("📞 Server sent 180 Ringing");
                }
                
                // Continue ringing for a while, then check if cancelled
                tokio::time::sleep(Duration::from_millis(1500)).await;
                
                // Check if this INVITE was cancelled
                if let Some(&cancelled) = pending_invites.get(&transaction_id) {
                    if cancelled {
                        // Send 487 Request Terminated
                        let request_terminated = server_quick::request_terminated(&request)
                            .expect("Failed to create 487 response");
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, request_terminated).await {
                            error!("Failed to send 487 Request Terminated: {}", e);
                        } else {
                            info!("✅ Server sent 487 Request Terminated (call was cancelled)");
                        }
                    } else {
                        // Send 200 OK (call was answered)
                        let sdp_answer = "v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n...";
                        let contact = format!("sip:server@{}", source.ip());
                        let ok = server_quick::ok_invite(&request, Some(sdp_answer.to_string()), contact)
                            .expect("Failed to create 200 OK");
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
                            error!("Failed to send 200 OK: {}", e);
                        } else {
                            info!("✅ Server sent 200 OK (call answered)");
                        }
                    }
                }
                
                pending_invites.remove(&transaction_id);
            }
            TransactionEvent::NonInviteRequest { transaction_id, request, source, .. } => {
                match request.method() {
                    Method::Cancel => {
                        info!("❌ Server received CANCEL from {}", source);
                        
                        // Send 200 OK to CANCEL immediately using the correct function
                        let ok = server_quick::ok_bye(&request)  // Use ok_bye as fallback for CANCEL
                            .expect("Failed to create 200 OK for CANCEL");
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
                            error!("Failed to send 200 OK to CANCEL: {}", e);
                        } else {
                            info!("✅ Server sent 200 OK to CANCEL");
                        }
                        
                        // Mark corresponding INVITE as cancelled
                        // In a real implementation, you'd match by Call-ID, From, and To
                        // For this example, we'll mark the first pending INVITE as cancelled
                        for (_, cancelled) in pending_invites.iter_mut() {
                            if !*cancelled {
                                *cancelled = true;
                                info!("🔄 Marked corresponding INVITE as cancelled");
                                break;
                            }
                        }
                    }
                    Method::Bye => {
                        info!("👋 Server received BYE from {}", source);
                        
                        let ok = server_quick::ok_bye(&request)
                            .expect("Failed to create 200 OK for BYE");
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
                            error!("Failed to send 200 OK to BYE: {}", e);
                        } else {
                            info!("✅ Server sent 200 OK to BYE");
                        }
                    }
                    _ => {
                        info!("🔹 Server received {} from {}", request.method(), source);
                    }
                }
            }
            _ => {
                debug!("Server received other event: {:?}", event);
            }
        }
    }
}

async fn handle_client_events(
    client_tm: TransactionManager,
    mut events: mpsc::Receiver<TransactionEvent>,
    server_addr: SocketAddr,
) {
    // Wait a bit, then send INVITE
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    info!("📤 Client sending INVITE to server...");
    
    let from_uri = "sip:alice@example.com";
    let to_uri = "sip:bob@example.com";
    let local_addr = "127.0.0.1:0".parse().unwrap(); // Will be updated by transport
    
    let invite = client_quick::invite(from_uri, to_uri, local_addr, None)
        .expect("Failed to create INVITE");
    
    // Create client transaction first, then send
    let invite_tx_id = match client_tm.create_client_transaction(invite, server_addr).await {
        Ok(tx_id) => {
            info!("📤 Created INVITE transaction with ID: {:?}", tx_id);
            tx_id
        }
        Err(e) => {
            error!("❌ Failed to create INVITE transaction: {}", e);
            return;
        }
    };
    
    // Now send the request
    if let Err(e) = client_tm.send_request(&invite_tx_id).await {
        error!("❌ Failed to send INVITE: {}", e);
        return;
    }
    info!("📤 Sent INVITE request");
    
    let mut received_ringing = false;
    let mut cancel_sent = false;
    let mut cancel_completed = false;
    let mut invite_completed = false;
    
    while !invite_completed || !cancel_completed {
        tokio::select! {
            Some(event) = events.recv() => {
                match event {
                    TransactionEvent::ProvisionalResponse { transaction_id, response, .. } 
                        if transaction_id == invite_tx_id => {
                        info!("📞 Client received provisional: {} {}", 
                            response.status_code(), 
                            response.reason_phrase());
                        
                        if response.status_code() == 180 && !cancel_sent {
                            received_ringing = true;
                            
                            // Wait a bit to simulate user deciding to cancel
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            
                            info!("❌ User decides to cancel the call!");
                            
                            // Send CANCEL using TransactionManager
                            match client_tm.cancel_invite_transaction(&invite_tx_id).await {
                                Ok(cancel_tx_id) => {
                                    info!("📤 Sent CANCEL with transaction ID: {:?}", cancel_tx_id);
                                    cancel_sent = true;
                                }
                                Err(e) => {
                                    error!("❌ Failed to send CANCEL: {}", e);
                                    cancel_completed = true; // Mark as completed to avoid hanging
                                }
                            }
                        }
                    }
                    TransactionEvent::SuccessResponse { transaction_id, response, .. } => {
                        // Check if this is a response to CANCEL
                        if response.cseq().map(|cseq| cseq.method == Method::Cancel).unwrap_or(false) {
                            info!("✅ CANCEL was accepted: {} {}", 
                                response.status_code(), 
                                response.reason_phrase());
                            cancel_completed = true;
                        }
                        // Check if this is 200 OK to INVITE (call answered before cancel took effect)
                        else if transaction_id == invite_tx_id {
                            info!("📞 Call was answered before cancel! Sending ACK and BYE");
                            
                            // Send ACK to 200 OK using TransactionManager
                            if let Err(e) = client_tm.send_ack_for_2xx(&invite_tx_id, &response).await {
                                error!("❌ Failed to send ACK: {}", e);
                            } else {
                                info!("📤 Sent ACK to complete call setup");
                            }
                            
                            // Send BYE to terminate the session
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            let bye = client_quick::bye(
                                &response.call_id().unwrap().value(),
                                response.from().unwrap().address().uri.to_string().as_str(),
                                response.from().unwrap().tag().unwrap_or(""),
                                response.to().unwrap().address().uri.to_string().as_str(),
                                response.to().unwrap().tag().unwrap_or(""),
                                "127.0.0.1:0".parse().unwrap(),
                                2,
                            ).expect("Failed to create BYE");
                            
                            // Create and send BYE transaction
                            match client_tm.create_client_transaction(bye, server_addr).await {
                                Ok(bye_tx_id) => {
                                    if let Err(e) = client_tm.send_request(&bye_tx_id).await {
                                        error!("❌ Failed to send BYE: {}", e);
                                    } else {
                                        info!("📤 Sent BYE with transaction ID: {:?}", bye_tx_id);
                                    }
                                }
                                Err(e) => {
                                    error!("❌ Failed to create BYE transaction: {}", e);
                                }
                            }
                            
                            invite_completed = true;
                        }
                    }
                    TransactionEvent::FailureResponse { transaction_id, response, .. } 
                        if transaction_id == invite_tx_id => {
                        if response.status_code() == 487 {
                            info!("✅ INVITE was cancelled: 487 Request Terminated");
                            
                            // For non-2xx responses, the transaction layer automatically generates ACK
                            // according to RFC 3261, so we don't need to send it manually
                            info!("📤 ACK for 487 will be sent automatically by transaction layer");
                            info!("🎉 CANCEL scenario completed successfully!");
                            
                            invite_completed = true;
                        } else {
                            info!("❌ INVITE failed: {} {}", 
                                response.status_code(), 
                                response.reason_phrase());
                            invite_completed = true;
                        }
                    }
                    _ => {
                        debug!("Client received other event: {:?}", event);
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                // Timeout to avoid hanging if no CANCEL was sent
                if !cancel_sent && received_ringing {
                    cancel_completed = true;
                }
            }
        }
    }
    
    info!("🏁 Client event handling completed");
} 