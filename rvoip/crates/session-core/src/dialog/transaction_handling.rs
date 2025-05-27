use tracing::{debug, error};
use std::net::SocketAddr;

use rvoip_sip_core::{Request, Method};
use rvoip_transaction_core::TransactionKey;

use super::manager::DialogManager;
use crate::events::SessionEvent;
use crate::session::SessionId;

impl DialogManager {
    /// Create a server transaction for a new request
    pub(super) async fn create_server_transaction_for_request(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr
    ) {
        match request.method() {
            Method::Invite => {
                self.handle_invite_server_transaction(transaction_id, request, source).await;
            },
            Method::Bye => {
                self.handle_bye_server_transaction(transaction_id, request, source).await;
            },
            _ => {
                self.handle_other_server_transaction(transaction_id, request, source).await;
            }
        }
    }
    
    /// Handle INVITE server transaction creation
    async fn handle_invite_server_transaction(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr
    ) {
        // **CHECK FOR RE-INVITE**: First check if this is a re-INVITE (in-dialog INVITE)
        debug!("Processing INVITE request - checking if it's a re-INVITE");
        if let Some(dialog_id) = self.find_dialog_for_request(&request) {
            debug!("Detected re-INVITE for existing dialog {}", dialog_id);
            
            // This is a re-INVITE - handle it as an in-dialog request
            match self.transaction_manager.create_server_transaction(request.clone(), source).await {
                Ok(server_tx) => {
                    let server_tx_id = server_tx.id().clone();
                    debug!("Created server transaction {} for re-INVITE", server_tx_id);
                    
                    // Associate this transaction with the existing dialog
                    self.transaction_to_dialog.insert(server_tx_id.clone(), dialog_id.clone());
                    
                    // Send 100 Trying immediately
                    let trying_response = rvoip_transaction_core::utils::create_trying_response(&request);
                    if let Err(e) = self.transaction_manager.send_response(&server_tx_id, trying_response).await {
                        error!("Failed to send 100 Trying for re-INVITE: {}", e);
                        return;
                    }
                    debug!("Sent 100 Trying for re-INVITE transaction {}", server_tx_id);
                    
                    // For re-INVITE, send 200 OK directly (no ringing phase)
                    let local_addr = self.transaction_manager.transport().local_addr()
                        .unwrap_or_else(|_| source);
                    
                    let server_user = "server"; // This should be configurable
                    let server_host = local_addr.ip().to_string();
                    let server_port = if local_addr.port() != 5060 { Some(local_addr.port()) } else { None };
                    
                    let ok_response = rvoip_transaction_core::utils::create_ok_response_with_dialog_info(
                        &request, 
                        server_user, 
                        &server_host, 
                        server_port
                    );
                    
                    if let Err(e) = self.transaction_manager.send_response(&server_tx_id, ok_response.clone()).await {
                        error!("Failed to send 200 OK for re-INVITE: {}", e);
                        return;
                    }
                    debug!("Sent 200 OK for re-INVITE transaction {} - dialog updated!", server_tx_id);
                    
                    // Emit event for re-INVITE (session-core can coordinate session update)
                    self.event_bus.publish(crate::events::SessionEvent::Custom {
                        session_id: SessionId::new(), // We don't know the session yet
                        event_type: "re_invite_processed".to_string(),
                        data: serde_json::json!({
                            "dialog_id": dialog_id.to_string(),
                            "transaction_id": server_tx_id.to_string(),
                            "original_transaction_id": transaction_id.to_string(),
                        }),
                    });
                },
                Err(e) => {
                    error!("Failed to create server transaction for re-INVITE: {}", e);
                }
            }
            return; // Important: return here to avoid treating as new call
        }
        
        // If we get here, this is an initial INVITE (no existing dialog found)
        debug!("No existing dialog found - treating as initial INVITE");
        
        // **ARCHITECTURAL FIX**: Create actual server transaction and send responses
        // This is what transaction-core examples do - we need to create the server transaction
        // and send the required SIP responses (100 Trying, 180 Ringing, 200 OK)
        
        // Create server transaction using transaction manager
        match self.transaction_manager.create_server_transaction(request.clone(), source).await {
            Ok(server_tx) => {
                let server_tx_id = server_tx.id().clone();
                debug!("Created server transaction {} for INVITE", server_tx_id);
                
                // Send 100 Trying immediately (required by RFC 3261)
                let trying_response = rvoip_transaction_core::utils::create_trying_response(&request);
                if let Err(e) = self.transaction_manager.send_response(&server_tx_id, trying_response).await {
                    error!("Failed to send 100 Trying response: {}", e);
                    return;
                }
                debug!("Sent 100 Trying for INVITE transaction {}", server_tx_id);
                
                // Wait a bit to simulate processing
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                
                // Send 180 Ringing
                let ringing_response = rvoip_transaction_core::utils::create_ringing_response_with_tag(&request);
                if let Err(e) = self.transaction_manager.send_response(&server_tx_id, ringing_response).await {
                    error!("Failed to send 180 Ringing response: {}", e);
                    return;
                }
                debug!("Sent 180 Ringing for INVITE transaction {}", server_tx_id);
                
                // Wait a bit more to simulate phone ringing
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                
                // **COMPLETE THE CALL FLOW**: Send 200 OK to accept the call
                // Use transaction-core's dialog-aware response builder with proper parameters
                // Get the local address from the transport instead of hardcoding
                let local_addr = self.transaction_manager.transport().local_addr()
                    .unwrap_or_else(|_| source);
                
                // Use a configurable server identity instead of hardcoded "server"
                // TODO: This should come from server configuration
                let server_user = "server"; // This should be configurable
                let server_host = local_addr.ip().to_string();
                let server_port = if local_addr.port() != 5060 { Some(local_addr.port()) } else { None };
                
                let ok_response = rvoip_transaction_core::utils::create_ok_response_with_dialog_info(
                    &request, 
                    server_user, 
                    &server_host, 
                    server_port
                );
                
                if let Err(e) = self.transaction_manager.send_response(&server_tx_id, ok_response.clone()).await {
                    error!("Failed to send 200 OK response: {}", e);
                    return;
                }
                debug!("Sent 200 OK for INVITE transaction {} - call established!", server_tx_id);
                
                // Create dialog from the INVITE transaction using the actual response we sent
                debug!("Attempting to create dialog from INVITE transaction {} with response status {}", 
                       server_tx_id, ok_response.status);
                if let Some(dialog_id) = self.create_dialog_from_transaction(&server_tx_id, &request, &ok_response, false).await {
                    debug!("Created dialog {} for established call", dialog_id);
                } else {
                    debug!("Failed to create dialog for INVITE transaction {}", server_tx_id);
                }
                
                // Emit event for new INVITE (session-core can coordinate session creation)
                self.event_bus.publish(crate::events::SessionEvent::Custom {
                    session_id: SessionId::new(), // We don't know the session yet
                    event_type: "call_established".to_string(),
                    data: serde_json::json!({
                        "transaction_id": server_tx_id.to_string(),
                        "original_transaction_id": transaction_id.to_string(),
                    }),
                });
            },
            Err(e) => {
                error!("Failed to create server transaction for INVITE: {}", e);
                
                // Emit error event
                self.event_bus.publish(crate::events::SessionEvent::Custom {
                    session_id: SessionId::new(),
                    event_type: "invite_error".to_string(),
                    data: serde_json::json!({
                        "error": e.to_string(),
                        "transaction_id": transaction_id.to_string(),
                    }),
                });
            }
        }
    }
    
    /// Handle BYE server transaction creation
    async fn handle_bye_server_transaction(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr
    ) {
        debug!("Creating server transaction for BYE request");
        
        // For BYE requests, create server transaction and send 200 OK
        match self.transaction_manager.create_server_transaction(request.clone(), source).await {
            Ok(server_tx) => {
                let server_tx_id = server_tx.id().clone();
                debug!("Created server transaction {} for BYE", server_tx_id);
                
                // Send 200 OK immediately for BYE
                let ok_response = rvoip_transaction_core::utils::create_ok_response(&request);
                if let Err(e) = self.transaction_manager.send_response(&server_tx_id, ok_response).await {
                    error!("Failed to send 200 OK for BYE: {}", e);
                    return;
                }
                debug!("Sent 200 OK for BYE transaction {} - call terminated", server_tx_id);
                
                // Find and terminate the associated dialog
                if let Some(dialog_id) = self.find_dialog_for_request(&request) {
                    debug!("Found dialog {} for BYE, terminating", dialog_id);
                    if let Err(e) = self.terminate_dialog(&dialog_id).await {
                        error!("Failed to terminate dialog {}: {}", dialog_id, e);
                    }
                }
                
                // Emit call terminated event
                self.event_bus.publish(crate::events::SessionEvent::Custom {
                    session_id: SessionId::new(),
                    event_type: "call_terminated".to_string(),
                    data: serde_json::json!({
                        "transaction_id": server_tx_id.to_string(),
                        "original_transaction_id": transaction_id.to_string(),
                    }),
                });
            },
            Err(e) => {
                error!("Failed to create server transaction for BYE: {}", e);
            }
        }
    }
    
    /// Handle other server transaction creation
    async fn handle_other_server_transaction(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr
    ) {
        debug!("Received {} request", request.method());
        
        // For other methods, create server transaction and send appropriate response
        match self.transaction_manager.create_server_transaction(request.clone(), source).await {
            Ok(server_tx) => {
                let server_tx_id = server_tx.id().clone();
                debug!("Created server transaction {} for {}", server_tx_id, request.method());
                
                // Send appropriate response based on method
                let response = match request.method() {
                    Method::Options => {
                        // Send 200 OK with capabilities
                        rvoip_transaction_core::utils::create_ok_response(&request)
                    },
                    Method::Info => {
                        // Send 200 OK
                        rvoip_transaction_core::utils::create_ok_response(&request)
                    },
                    Method::Message => {
                        // Send 200 OK
                        rvoip_transaction_core::utils::create_ok_response(&request)
                    },
                    _ => {
                        // Send 200 OK for unknown methods
                        rvoip_transaction_core::utils::create_ok_response(&request)
                    }
                };
                
                if let Err(e) = self.transaction_manager.send_response(&server_tx_id, response).await {
                    error!("Failed to send response for {} request: {}", request.method(), e);
                } else {
                    debug!("Sent 200 OK for {} transaction {}", request.method(), server_tx_id);
                }
                
                // Emit event for the request
                self.event_bus.publish(crate::events::SessionEvent::Custom {
                    session_id: SessionId::new(), // We don't know the session yet
                    event_type: format!("new_{}", request.method().to_string().to_lowercase()),
                    data: serde_json::json!({
                        "transaction_id": server_tx_id.to_string(),
                        "original_transaction_id": transaction_id.to_string(),
                    }),
                });
            },
            Err(e) => {
                error!("Failed to create server transaction for {}: {}", request.method(), e);
            }
        }
    }
} 