/**
 * Dialog Integration Example
 * 
 * This example demonstrates the new dialog integration features added in Phase 3 
 * of the dialog-transaction integration project. It showcases:
 *
 * 1. Dialog utility functions for bridging dialog-core templates and transaction-core builders
 * 2. Quick dialog functions for one-liner dialog operations
 * 3. Dialog-aware request and response building
 * 4. Complete dialog flow using the enhanced integration
 *
 * The example demonstrates these key Phase 3 features:
 * - DialogRequestTemplate and DialogTransactionContext
 * - request_builder_from_dialog_template() and response_builder_for_dialog_transaction()
 * - Quick functions: bye_for_dialog(), refer_for_dialog(), update_for_dialog(), etc.
 * - Seamless integration between dialog context and transaction builders
 *
 * To run this example with full logging:
 * ```
 * RUST_LOG=rvoip=debug cargo run --example dialog_example
 * ```
 */

use std::net::SocketAddr;
use std::time::Duration;

use rvoip_sip_core::{Method, StatusCode};
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionState, TransactionKey};
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
use rvoip_transaction_core::builders::{client_quick, server_quick, dialog_utils, dialog_quick};
use rvoip_transaction_core::dialog::{DialogRequestTemplate, DialogTransactionContext};

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup logging
    tracing_subscriber::fmt()
        .with_env_filter("rvoip=debug")
        .init();
    
    // ------------- Server setup -----------------
    
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
    
    let server_addr = server_transport.default_transport().await
        .ok_or("No default transport")?.local_addr()?;
        
    info!("🟡 Server bound to {}", server_addr);
    
    let (server_tm, mut server_events) = TransactionManager::with_transport_manager(
        server_transport.clone(),
        server_transport_rx,
        Some(100),
    ).await?;
    
    // ------------- Client setup -----------------
    
    let client_config = TransportManagerConfig {
        enable_udp: true,
        enable_tcp: false,
        enable_ws: false,
        enable_tls: false,
        bind_addresses: vec!["127.0.0.1:0".parse()?], // Ephemeral port
        ..Default::default()
    };
    
    let (mut client_transport, client_transport_rx) = TransportManager::new(client_config).await?;
    client_transport.initialize().await?;
    
    let client_addr = client_transport.default_transport().await
        .ok_or("No default transport")?.local_addr()?;
        
    info!("🔵 Client bound to {}", client_addr);
    
    let (client_tm, mut client_events) = TransactionManager::with_transport_manager(
        client_transport.clone(),
        client_transport_rx,
        Some(100),
    ).await?;
    
    // ------------- Server event handler -----------------
    
    tokio::spawn(handle_server_events_with_dialog_functions(server_tm.clone(), server_events));
    
    // ------------- EXAMPLE 1: Dialog Utility Functions -----------------
    
    info!("🎯 EXAMPLE 1: Dialog Utility Functions");
    
    // Create initial INVITE to establish dialog
    let from_uri = format!("sip:alice@{}", client_addr.ip());
    let to_uri = format!("sip:bob@{}", server_addr.ip());
    let sdp_content = "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5004 RTP/AVP 0\r\n";
    
    let initial_invite = client_quick::invite(&from_uri, &to_uri, client_addr, Some(sdp_content))
        .expect("Failed to create initial INVITE");
    
    let call_id = initial_invite.call_id().unwrap().value().to_string();
    let from_tag = initial_invite.from().unwrap().tag().unwrap().to_string();
    
    // Send initial INVITE
    let invite_tx_id = client_tm.create_client_transaction(initial_invite.clone(), server_addr).await?;
    let mut invite_events = client_tm.subscribe_to_transaction(&invite_tx_id).await?;
    client_tm.send_request(&invite_tx_id).await?;
    info!("📤 Sent initial INVITE to establish dialog");
    
    // Wait for 200 OK response to get to_tag
    let mut to_tag = None;
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    
    while to_tag.is_none() && start.elapsed() < timeout {
        tokio::select! {
            Some(event) = invite_events.recv() => {
                if let TransactionEvent::SuccessResponse { response, .. } = event {
                    to_tag = response.to().and_then(|t| t.tag()).map(|tag| tag.to_string());
                    info!("✅ Received 200 OK - dialog established with to_tag: {:?}", to_tag);
                    break;
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    
    let to_tag = to_tag.ok_or("Failed to establish dialog - no to_tag received")?;
    
    // Give server time to process
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Now demonstrate dialog utility functions
    info!("🔧 Using Dialog Utility Functions:");
    
    // 1. Create DialogRequestTemplate for subsequent requests
    let dialog_template = DialogRequestTemplate {
        call_id: call_id.clone(),
        from_uri: from_uri.clone(),
        from_tag: from_tag.clone(),
        to_uri: to_uri.clone(),
        to_tag: to_tag.clone(),
        request_uri: to_uri.clone(),
        cseq: 2,
        local_address: client_addr,
        route_set: vec![],
        contact: None,
    };
    
    // 2. Use request_builder_from_dialog_template to create UPDATE
    let update_request = dialog_utils::request_builder_from_dialog_template(
        &dialog_template,
        Method::Update,
        Some("v=0\r\no=alice 789 012 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5006 RTP/AVP 0\r\n".to_string()),
        Some("application/sdp".to_string())
    ).expect("Failed to create UPDATE from dialog template");
    
    info!("📝 Created UPDATE request using dialog template");
    
    // Send UPDATE
    let update_tx_id = client_tm.create_client_transaction(update_request, server_addr).await?;
    let mut update_events = client_tm.subscribe_to_transaction(&update_tx_id).await?;
    client_tm.send_request(&update_tx_id).await?;
    info!("📤 Sent UPDATE request using dialog utility function");
    
    // Wait for UPDATE response
    wait_for_transaction_completion(&mut update_events, &update_tx_id, "UPDATE").await;
    
    // 3. Use extract_dialog_template_from_request
    let extracted_template = dialog_utils::extract_dialog_template_from_request(
        &initial_invite,
        client_addr,
        3
    ).expect("Failed to extract dialog template from request");
    
    info!("🔍 Extracted dialog template from original INVITE request");
    assert_eq!(extracted_template.call_id, call_id);
    assert_eq!(extracted_template.from_uri, from_uri);
    
    // ------------- EXAMPLE 2: Quick Dialog Functions -----------------
    
    info!("🎯 EXAMPLE 2: Quick Dialog Functions (One-Liners)");
    
    // 1. Quick REFER for call transfer
    let refer_request = dialog_quick::refer_for_dialog(
        &call_id,
        &from_uri,
        &from_tag,
        &to_uri,
        &to_tag,
        "sip:charlie@example.com", // Transfer target
        4,
        client_addr,
        None
    ).expect("Failed to create REFER with quick function");
    
    let refer_tx_id = client_tm.create_client_transaction(refer_request, server_addr).await?;
    let mut refer_events = client_tm.subscribe_to_transaction(&refer_tx_id).await?;
    client_tm.send_request(&refer_tx_id).await?;
    info!("📤 Sent REFER request using dialog_quick::refer_for_dialog()");
    
    wait_for_transaction_completion(&mut refer_events, &refer_tx_id, "REFER").await;
    
    // 2. Quick INFO for mid-dialog information
    let info_request = dialog_quick::info_for_dialog(
        &call_id,
        &from_uri,
        &from_tag,
        &to_uri,
        &to_tag,
        "Custom application data from client",
        Some("application/custom".to_string()),
        5,
        client_addr,
        None
    ).expect("Failed to create INFO with quick function");
    
    let info_tx_id = client_tm.create_client_transaction(info_request, server_addr).await?;
    let mut info_events = client_tm.subscribe_to_transaction(&info_tx_id).await?;
    client_tm.send_request(&info_tx_id).await?;
    info!("📤 Sent INFO request using dialog_quick::info_for_dialog()");
    
    wait_for_transaction_completion(&mut info_events, &info_tx_id, "INFO").await;
    
    // 3. Quick NOTIFY for event notifications
    let notify_request = dialog_quick::notify_for_dialog(
        &call_id,
        &from_uri,
        &from_tag,
        &to_uri,
        &to_tag,
        "dialog", // Event type
        Some("Dialog state: active".to_string()),
        6,
        client_addr,
        None
    ).expect("Failed to create NOTIFY with quick function");
    
    let notify_tx_id = client_tm.create_client_transaction(notify_request, server_addr).await?;
    let mut notify_events = client_tm.subscribe_to_transaction(&notify_tx_id).await?;
    client_tm.send_request(&notify_tx_id).await?;
    info!("📤 Sent NOTIFY request using dialog_quick::notify_for_dialog()");
    
    wait_for_transaction_completion(&mut notify_events, &notify_tx_id, "NOTIFY").await;
    
    // 4. Quick MESSAGE for instant messaging
    let message_request = dialog_quick::message_for_dialog(
        &call_id,
        &from_uri,
        &from_tag,
        &to_uri,
        &to_tag,
        "Hello from Alice! This is sent using the dialog quick functions.",
        Some("text/plain".to_string()),
        7,
        client_addr,
        None
    ).expect("Failed to create MESSAGE with quick function");
    
    let message_tx_id = client_tm.create_client_transaction(message_request, server_addr).await?;
    let mut message_events = client_tm.subscribe_to_transaction(&message_tx_id).await?;
    client_tm.send_request(&message_tx_id).await?;
    info!("📤 Sent MESSAGE request using dialog_quick::message_for_dialog()");
    
    wait_for_transaction_completion(&mut message_events, &message_tx_id, "MESSAGE").await;
    
    // 5. Quick re-INVITE for session modification
    let reinvite_request = dialog_quick::reinvite_for_dialog(
        &call_id,
        &from_uri,
        &from_tag,
        &to_uri,
        &to_tag,
        "v=0\r\no=alice 890 123 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5008 RTP/AVP 0\r\n",
        8,
        client_addr,
        None,
        Some(format!("sip:alice@{}", client_addr.ip()))
    ).expect("Failed to create re-INVITE with quick function");
    
    let reinvite_tx_id = client_tm.create_client_transaction(reinvite_request, server_addr).await?;
    let mut reinvite_events = client_tm.subscribe_to_transaction(&reinvite_tx_id).await?;
    client_tm.send_request(&reinvite_tx_id).await?;
    info!("📤 Sent re-INVITE request using dialog_quick::reinvite_for_dialog()");
    
    wait_for_transaction_completion(&mut reinvite_events, &reinvite_tx_id, "re-INVITE").await;
    
    // 6. Finally, terminate with quick BYE
    let bye_request = dialog_quick::bye_for_dialog(
        &call_id,
        &from_uri,
        &from_tag,
        &to_uri,
        &to_tag,
        9,
        client_addr,
        None
    ).expect("Failed to create BYE with quick function");
    
    let bye_tx_id = client_tm.create_client_transaction(bye_request, server_addr).await?;
    let mut bye_events = client_tm.subscribe_to_transaction(&bye_tx_id).await?;
    client_tm.send_request(&bye_tx_id).await?;
    info!("📤 Sent BYE request using dialog_quick::bye_for_dialog()");
    
    wait_for_transaction_completion(&mut bye_events, &bye_tx_id, "BYE").await;
    
    info!("🎉 Dialog integration example completed successfully!");
    info!("✅ All Phase 3 dialog functions demonstrated:");
    info!("   - Dialog utility functions (DialogRequestTemplate, response building)");  
    info!("   - Quick dialog functions (one-liners for all SIP methods)");
    info!("   - Seamless dialog-transaction integration");
    
    // Clean up
    tokio::time::sleep(Duration::from_millis(500)).await;
    client_tm.shutdown().await;
    server_tm.shutdown().await;
    
    Ok(())
}

async fn wait_for_transaction_completion(
    events: &mut mpsc::Receiver<TransactionEvent>,
    tx_id: &TransactionKey,
    method_name: &str
) {
    let timeout = Duration::from_secs(3);
    let start = std::time::Instant::now();
    
    while start.elapsed() < timeout {
        tokio::select! {
            Some(event) = events.recv() => {
                match event {
                    TransactionEvent::SuccessResponse { transaction_id, response, .. } 
                        if transaction_id == *tx_id => {
                        info!("✅ {} received response: {} {}", 
                              method_name, response.status_code(), response.reason_phrase());
                        return;
                    },
                    TransactionEvent::FailureResponse { transaction_id, response }
                        if transaction_id == *tx_id => {
                        info!("⚠️  {} received failure: {} {}", 
                              method_name, response.status_code(), response.reason_phrase());
                        return;
                    },
                    TransactionEvent::StateChanged { transaction_id, new_state, .. }
                        if transaction_id == *tx_id && 
                           (new_state == TransactionState::Completed || new_state == TransactionState::Terminated) => {
                        info!("✅ {} transaction completed", method_name);
                        return;
                    },
                    _ => {}
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    
    warn!("⚠️  {} transaction timed out", method_name);
}

async fn handle_server_events_with_dialog_functions(
    server_tm: TransactionManager,
    mut events: mpsc::Receiver<TransactionEvent>,
) {
    info!("🟡 Server event handler started - demonstrating dialog response functions");
    
    while let Some(event) = events.recv().await {
        match event {
            TransactionEvent::InviteRequest { transaction_id, request, source, .. } => {
                info!("🟡 Server received INVITE from {}", source);
                
                // Send 180 Ringing first
                tokio::time::sleep(Duration::from_millis(100)).await;
                let ringing = server_quick::ringing(&request, None)
                    .expect("Failed to create 180 Ringing");
                
                if let Err(e) = server_tm.send_response(&transaction_id, ringing).await {
                    error!("Failed to send 180 Ringing: {}", e);
                    continue;
                }
                info!("📞 Server sent 180 Ringing");
                
                // Send 200 OK with SDP - using dialog-aware response
                tokio::time::sleep(Duration::from_millis(300)).await;
                
                // Demonstrate dialog-aware response building using utility functions
                let dialog_context = dialog_utils::create_dialog_transaction_context(
                    transaction_id.to_string(),
                    request.clone(),
                    Some("server-dialog-123".to_string()),
                    source
                );
                
                let ok_response = dialog_utils::response_builder_for_dialog_transaction(
                    &dialog_context,
                    StatusCode::Ok,
                    Some(source),
                    Some("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
                ).expect("Failed to create dialog-aware OK response");
                
                if let Err(e) = server_tm.send_response(&transaction_id, ok_response).await {
                    error!("Failed to send 200 OK: {}", e);
                } else {
                    info!("✅ Server sent 200 OK using dialog_utils::response_builder_for_dialog_transaction()");
                }
            },
            TransactionEvent::AckRequest { transaction_id, .. } => {
                info!("📨 Server received ACK for transaction {}", transaction_id);
            },
            TransactionEvent::NonInviteRequest { transaction_id, request, source, .. } => {
                info!("🟡 Server received {} from {}", request.method(), source);
                
                // Use appropriate response functions for all non-INVITE requests
                let response = match request.method() {
                    Method::Update => {
                        info!("🔄 Processing UPDATE with SDP");
                        // For UPDATE, send 200 OK with SDP
                        use rvoip_transaction_core::server::builders::ResponseBuilder;
                        ResponseBuilder::new(StatusCode::Ok)
                            .from_request(&request)
                            .with_sdp("v=0\r\no=server 678 901 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5006 RTP/AVP 0\r\n")
                            .build()
                    },
                    Method::Refer => {
                        info!("🔀 Processing REFER for call transfer");
                        // For REFER, send 202 Accepted
                        use rvoip_transaction_core::server::builders::ResponseBuilder;
                        ResponseBuilder::new(StatusCode::Accepted)
                            .from_request(&request)
                            .build()
                    },
                    Method::Info => {
                        info!("ℹ️  Processing INFO request");
                        use rvoip_transaction_core::server::builders::ResponseBuilder;
                        ResponseBuilder::new(StatusCode::Ok)
                            .from_request(&request)
                            .build()
                    },
                    Method::Notify => {
                        info!("🔔 Processing NOTIFY request");
                        use rvoip_transaction_core::server::builders::ResponseBuilder;
                        ResponseBuilder::new(StatusCode::Ok)
                            .from_request(&request)
                            .build()
                    },
                    Method::Message => {
                        info!("💬 Processing MESSAGE request");
                        // Log the message content
                        let message_body = String::from_utf8_lossy(request.body());
                        info!("📥 Received message: {}", message_body);
                        use rvoip_transaction_core::server::builders::ResponseBuilder;
                        ResponseBuilder::new(StatusCode::Ok)
                            .from_request(&request)
                            .build()
                    },
                    Method::Bye => {
                        info!("👋 Processing BYE - terminating dialog");
                        server_quick::ok_bye(&request)
                    },
                    _ => {
                        warn!("❓ Unexpected method: {}", request.method());
                        // For unsupported methods, send 405 Method Not Allowed
                        use rvoip_transaction_core::server::builders::ResponseBuilder;
                        ResponseBuilder::new(StatusCode::MethodNotAllowed)
                            .from_request(&request)
                            .build()
                    }
                }.expect("Failed to create response");
                
                if let Err(e) = server_tm.send_response(&transaction_id, response).await {
                    error!("Failed to send response: {}", e);
                } else {
                    info!("✅ Server sent response for {}", request.method());
                }
            },
            other_event => {
                debug!("🔄 Server received other event: {:?}", other_event);
            }
        }
    }
    
    info!("🛑 Server event handler shutting down");
} 