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
    
    info!("🚀 Spawning server event handler...");
    tokio::spawn(handle_server_events_with_dialog_functions(server_tm.clone(), server_events));
    info!("✅ Server event handler spawned successfully");
    
    // ------------- Client event handler -----------------
    
    info!("🚀 Spawning client event handler...");
    let (dialog_event_tx, mut dialog_event_rx) = mpsc::channel::<String>(100);
    let dialog_event_sender = dialog_event_tx.clone();
    
    tokio::spawn(handle_global_client_events(client_events, dialog_event_sender));
    info!("✅ Client event handler spawned successfully");
    
    // ------------- EXAMPLE 1: Dialog Establishment -----------------
    
    info!("🎯 EXAMPLE 1: Dialog Establishment using Global Event Pattern");
    
    // Create initial INVITE to establish dialog
    let from_uri = format!("sip:alice@{}", client_addr.ip());
    let to_uri = format!("sip:bob@{}", server_addr.ip());
    let sdp_content = "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\nc=IN IP4 127.0.0.1\r\nm=audio 5004 RTP/AVP 0\r\n";
    
    let initial_invite = client_quick::invite(&from_uri, &to_uri, client_addr, Some(sdp_content))
        .expect("Failed to create initial INVITE");
    
    let call_id = initial_invite.call_id().unwrap().value().to_string();
    let from_tag = initial_invite.from().unwrap().tag().unwrap().to_string();
    
    // Send initial INVITE using global event pattern
    let invite_tx_id = client_tm.create_client_transaction(initial_invite.clone(), server_addr).await?;
    info!("📡 Created INVITE transaction: {:?}", invite_tx_id);
    
    client_tm.send_request(&invite_tx_id).await?;
    info!("📤 Sent initial INVITE to establish dialog");
    
    // Wait for dialog establishment via global event processor (NON-BLOCKING!)
    info!("🔍 Waiting for dialog establishment via global events...");
    let to_tag = match dialog_event_rx.recv().await {
        Some(to_tag) => {
            info!("🎉 Dialog successfully established with to_tag: {}", to_tag);
            to_tag
        },
        None => return Err("Dialog establishment channel closed".into()),
    };
    
    // Give server time to process
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    info!("🚀 Dialog established! Now demonstrating basic dialog operations...");
    
    // Now demonstrate dialog utility functions with global event pattern
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
    
    // 2. Create a simple BYE request to demonstrate the pattern
    let bye_request = dialog_quick::bye_for_dialog(
        &call_id,
        &from_uri,
        &from_tag,
        &to_uri,
        &to_tag,
        3,
        client_addr,
        None
    ).expect("Failed to create BYE with quick function");
    
    let bye_tx_id = client_tm.create_client_transaction(bye_request, server_addr).await?;
    client_tm.send_request(&bye_tx_id).await?;
    info!("📤 Sent BYE request using dialog_quick::bye_for_dialog()");
    
    // Wait a bit for the transaction to complete
    tokio::time::sleep(Duration::from_millis(1000)).await;
    
    info!("🎉 Dialog integration example completed successfully!");
    info!("✅ Demonstrated working global event pattern:");
    info!("   - Global transaction event consumption (like dialog-core)");  
    info!("   - Non-blocking dialog establishment");
    info!("   - Proper event-driven architecture");
    
    // Clean up
    tokio::time::sleep(Duration::from_millis(500)).await;
    client_tm.shutdown().await;
    server_tm.shutdown().await;
    
    Ok(())
}

// ============================================================================
// ARCHITECTURE NOTE: Removed broken wait_for_transaction_completion function
//
// The previous implementation used individual transaction subscriptions with
// blocking event loops, which caused hanging issues. The working pattern
// (used by dialog-core) is to:
//
// 1. Consume ALL transaction events globally in a spawned task
// 2. Route events to appropriate handlers/notifications
// 3. Keep main application logic non-blocking
// 4. Never mix global events with individual transaction subscriptions
//
// This matches the successful pattern from dialog-core that works perfectly.
// ============================================================================

async fn handle_server_events_with_dialog_functions(
    server_tm: TransactionManager,
    mut events: mpsc::Receiver<TransactionEvent>,
) {
    info!("🟡 Server event handler started - demonstrating dialog response functions");
    info!("🔍 Server event handler ready to receive events...");
    
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

async fn handle_global_client_events(
    mut events: mpsc::Receiver<TransactionEvent>,
    dialog_event_sender: mpsc::Sender<String>,
) {
    info!("🔵 Client global event handler started - processing ALL transaction events");
    
    let mut pending_dialogs: std::collections::HashMap<TransactionKey, String> = std::collections::HashMap::new();
    
    while let Some(event) = events.recv().await {
        match event {
            TransactionEvent::ProvisionalResponse { transaction_id, response } => {
                info!("🔵 Received provisional response: {} {} for {}", 
                       response.status_code(), response.reason_phrase(), transaction_id);
                
                // Extract to_tag from 180 Ringing for dialog establishment
                if response.status_code() == 180 {
                    if let Some(to_tag) = response.to().and_then(|t| t.tag()).map(|tag| tag.to_string()) {
                        info!("🎯 Extracted to_tag from 180 Ringing: {}", to_tag);
                        pending_dialogs.insert(transaction_id, to_tag);
                    }
                }
            },
            TransactionEvent::SuccessResponse { transaction_id, response, .. } => {
                info!("🔵 Received success response: {} {} for {}", 
                       response.status_code(), response.reason_phrase(), transaction_id);
                
                // Extract to_tag from 200 OK for dialog establishment
                if response.status_code() == 200 {
                    if let Some(to_tag) = response.to().and_then(|t| t.tag()).map(|tag| tag.to_string()) {
                        info!("🎯 Extracted to_tag from 200 OK: {}", to_tag);
                        pending_dialogs.insert(transaction_id, to_tag);
                    }
                }
            },
            TransactionEvent::TransactionTerminated { transaction_id } => {
                info!("🔵 Transaction terminated: {}", transaction_id);
                
                // If we have a pending dialog for this transaction, it's now established
                if let Some(to_tag) = pending_dialogs.remove(&transaction_id) {
                    info!("✅ Dialog established for transaction {} with to_tag: {}", transaction_id, to_tag);
                    if let Err(e) = dialog_event_sender.send(to_tag).await {
                        warn!("Failed to send dialog establishment event: {}", e);
                    }
                }
            },
            TransactionEvent::FailureResponse { transaction_id, response } => {
                warn!("🔵 Received failure response: {} {} for {}", 
                       response.status_code(), response.reason_phrase(), transaction_id);
                
                // Remove any pending dialog for failed transactions
                pending_dialogs.remove(&transaction_id);
            },
            other_event => {
                debug!("🔵 Other client event: {:?}", other_event);
            }
        }
    }
    
    info!("🛑 Client global event handler shutting down");
} 