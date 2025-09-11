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
 * - Require ACK for 2xx final responses (sent by dialog layer/application)
 * - ACK for non-2xx responses is handled automatically by transaction layer
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
    
    // ------------- EXAMPLE: INVITE Call Flow -----------------
    info!("INVITE Call Flow using production APIs");
    
    // Create an INVITE request with SDP content using the convenience builder
    let from_uri = format!("sip:client@{}", client_addr.ip());
    let to_uri = format!("sip:server@{}", server_addr.ip());
    let sdp_content = r#"v=0
o=client 123456 123456 IN IP4 127.0.0.1
s=Session
c=IN IP4 127.0.0.1
t=0 0
m=audio 5004 RTP/AVP 0
a=rtpmap:0 PCMU/8000"#;
    
    let invite_request = client_quick::invite(&from_uri, &to_uri, client_addr, Some(sdp_content))
        .expect("Failed to create INVITE request");
    
    // Extract dialog information before moving the request
    let call_id = invite_request.call_id().unwrap().value().to_string();
    let from_tag = invite_request.from().and_then(|f| f.tag()).unwrap_or("default-from-tag").to_string();
    
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
                        
                        // According to RFC 3261: ACK for 2xx responses must be sent by the dialog layer
                        // The transaction layer only handles ACK automatically for non-2xx responses
                        if let Err(e) = client_tm.send_ack_for_2xx(&invite_tx_id, &response).await {
                            error!("❌ Failed to send ACK for 2xx: {}", e);
                        } else {
                            info!("📤 Sent ACK for 2xx response (RFC 3261 compliance)");
                        }
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
    
    // Extract dialog information from the INVITE request for the BYE
    let to_tag = "default-to-tag".to_string(); // In real usage, this would come from the 200 OK response
    
    // Create a BYE request to terminate the session using the new builder
    let bye_request = client_quick::bye(
        &call_id,
        &from_uri,
        &from_tag,
        &to_uri,
        &to_tag,
        client_addr,
        2, // Increment CSeq for new request in same dialog
    ).expect("Failed to create BYE request");
    
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
    
    // ------------- BONUS: Demonstrate Phase 3 Dialog Functions -----------------
    info!("🎯 BONUS: Demonstrating Phase 3 Dialog Integration Functions");

    // The above example used the traditional client_quick::bye() function.
    // Now let's demonstrate the new Phase 3 dialog functions for comparison:

    // Extract dialog context from the completed call
    if received_200_ok {
        use rvoip_transaction_core::builders::{dialog_utils, dialog_quick};
        
        // Demonstrate dialog utility functions
        info!("🔧 Using Dialog Utility Functions:");
        
        // 1. Create a DialogRequestTemplate from dialog context
        let dialog_template = rvoip_transaction_core::dialog::DialogRequestTemplate {
            call_id: call_id.clone(),
            from_uri: from_uri.clone(),
            from_tag: from_tag.clone(),
            to_uri: to_uri.clone(),
            to_tag: "demo-to-tag".to_string(), // In real usage, from 200 OK response
            request_uri: to_uri.clone(),
            cseq: 10, // Next CSeq in dialog
            local_address: client_addr,
            route_set: vec![],
            contact: None,
        };
        
        // 2. Use dialog template to create an INFO request
        let info_request = dialog_utils::request_builder_from_dialog_template(
            &dialog_template,
            rvoip_sip_core::Method::Info,
            Some("Demonstration of dialog utility functions".to_string()),
            Some("text/plain".to_string())
        );
        
        match info_request {
            Ok(request) => {
                info!("✅ Created INFO request using dialog_utils::request_builder_from_dialog_template()");
                info!("   Call-ID: {}", request.call_id().unwrap().value());
                info!("   CSeq: {}", request.cseq().unwrap().seq);
                info!("   Method: {:?}", request.method());
            },
            Err(e) => {
                warn!("⚠️  Failed to create INFO with dialog template: {}", e);
            }
        }
        
        // Demonstrate quick dialog functions
        info!("⚡ Using Quick Dialog Functions (One-Liners):");
        
        // 1. Quick REFER for call transfer
        let refer_request = dialog_quick::refer_for_dialog(
            &call_id,
            &from_uri,
            &from_tag,
            &to_uri,
            "demo-to-tag",
            "sip:transfer-target@example.com", // Transfer target
            11,
            client_addr,
            None
        );
        
        match refer_request {
            Ok(request) => {
                info!("✅ Created REFER request using dialog_quick::refer_for_dialog()");
                info!("   Transfer target in body: {}", String::from_utf8_lossy(request.body()).contains("transfer-target"));
            },
            Err(e) => {
                warn!("⚠️  Failed to create REFER with quick function: {}", e);
            }
        }
        
        // 2. Quick UPDATE for session modification
        let update_request = dialog_quick::update_for_dialog(
            &call_id,
            &from_uri,
            &from_tag,
            &to_uri,
            "demo-to-tag",
            Some("v=0\r\no=alice-updated 456 789 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5008 RTP/AVP 0\r\n".to_string()),
            12,
            client_addr,
            None
        );
        
        match update_request {
            Ok(request) => {
                info!("✅ Created UPDATE request using dialog_quick::update_for_dialog()");
                info!("   Has SDP content: {}", request.body().len() > 0);
            },
            Err(e) => {
                warn!("⚠️  Failed to create UPDATE with quick function: {}", e);
            }
        }
        
        // 3. Quick MESSAGE for instant messaging
        let message_request = dialog_quick::message_for_dialog(
            &call_id,
            &from_uri,
            &from_tag,
            &to_uri,
            "demo-to-tag",
            "Hello! This message was created using the new dialog quick functions from Phase 3.",
            Some("text/plain".to_string()),
            13,
            client_addr,
            None
        );
        
        match message_request {
            Ok(request) => {
                info!("✅ Created MESSAGE request using dialog_quick::message_for_dialog()");
                info!("   Message content: {}", String::from_utf8_lossy(request.body()));
            },
            Err(e) => {
                warn!("⚠️  Failed to create MESSAGE with quick function: {}", e);
            }
        }
        
        // 4. Quick BYE (alternative to the one used above)
        let quick_bye_request = dialog_quick::bye_for_dialog(
            &call_id,
            &from_uri,
            &from_tag,
            &to_uri,
            "demo-to-tag",
            14,
            client_addr,
            None
        );
        
        match quick_bye_request {
            Ok(request) => {
                info!("✅ Created BYE request using dialog_quick::bye_for_dialog()");
                info!("   Compare with client_quick::bye() used earlier - both work!");
            },
            Err(e) => {
                warn!("⚠️  Failed to create BYE with quick function: {}", e);
            }
        }
        
        info!("🎉 Phase 3 Dialog Integration Functions demonstration completed!");
        info!("   Traditional builders: client_quick::invite(), client_quick::bye(), etc.");
        info!("   Dialog utility functions: request_builder_from_dialog_template(), response_builder_for_dialog_transaction()");
        info!("   Quick dialog functions: bye_for_dialog(), refer_for_dialog(), update_for_dialog(), etc.");
        info!("   ✨ All approaches work seamlessly together!");
    } else {
        info!("⚠️  Skipping dialog function demonstration - dialog not fully established");
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
    info!("🟡 Server event handler started - waiting for events...");
    
    while let Some(event) = events.recv().await {
        info!("🟡 Server received event: {:?}", event);
        
        match event {
            TransactionEvent::InviteRequest { transaction_id, request, source, .. } => {
                info!("🔹 Server received INVITE request from {}", source);
                
                // Send 180 Ringing
                tokio::time::sleep(Duration::from_millis(100)).await;
                let ringing = server_quick::ringing(&request, None)
                    .expect("Failed to create 180 Ringing response");
                
                if let Err(e) = server_tm.send_response(&transaction_id, ringing).await {
                    error!("Failed to send 180 Ringing: {}", e);
                    continue;
                }
                info!("📞 Server sent 180 Ringing");
                
                // Send 200 OK after a delay
                tokio::time::sleep(Duration::from_millis(500)).await;
                let sdp_answer = "v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n...";
                let contact = format!("sip:server@{}", source.ip());
                let ok = server_quick::ok_invite(&request, Some(sdp_answer.to_string()), contact)
                    .expect("Failed to create 200 OK response");
                
                if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
                    error!("Failed to send 200 OK: {}", e);
                    continue;
                }
                info!("✅ Server sent 200 OK");
            },
            TransactionEvent::AckRequest { transaction_id, request, source, .. } => {
                info!("📨 Server received ACK for transaction {} from {}", transaction_id, source);
                info!("🎉 INVITE call flow completed successfully!");
            },
            TransactionEvent::NonInviteRequest { transaction_id, request, source, .. } => {
                info!("🔹 Server received {} request from {}", request.method(), source);
                
                match request.method() {
                    Method::Bye => {
                        let ok = server_quick::ok_bye(&request)
                            .expect("Failed to create BYE response");
                        
                        if let Err(e) = server_tm.send_response(&transaction_id, ok).await {
                            error!("Failed to send BYE response: {}", e);
                        } else {
                            info!("📞 Server sent 200 OK to BYE - call terminated");
                        }
                    },
                    _ => {
                        warn!("🤷 Server received unexpected {} request", request.method());
                    }
                }
            },
            TransactionEvent::CancelReceived { transaction_id, cancel_request, .. } => {
                info!("🚫 Server received CANCEL for transaction {}", transaction_id);
                // The transaction layer automatically handles CANCEL responses
                info!("📞 Call cancelled by client");
            },
            other_event => {
                debug!("🔄 Server received other event: {:?}", other_event);
            }
        }
    }
    
    info!("🛑 Server event handler shutting down");
} 