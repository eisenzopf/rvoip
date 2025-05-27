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
            
            // This is a re-INVITE - associate with existing dialog
            self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
            
            // Emit event for re-INVITE (session-core can coordinate session update)
            self.event_bus.publish(crate::events::SessionEvent::Custom {
                session_id: SessionId::new(), // We don't know the session yet
                event_type: "re_invite_processed".to_string(),
                data: serde_json::json!({
                    "dialog_id": dialog_id.to_string(),
                    "transaction_id": transaction_id.to_string(),
                }),
            });
            
            return; // Important: return here to avoid treating as new call
        }
        
        // If we get here, this is an initial INVITE (no existing dialog found)
        debug!("No existing dialog found - treating as initial INVITE");
        
        // **ARCHITECTURAL FIX**: DialogManager should NOT create SIP responses!
        // That's transaction-core's responsibility. We only coordinate dialog state.
        // transaction-core will automatically:
        // 1. Send 100 Trying (if TU doesn't respond within 200ms)
        // 2. Handle retransmissions
        // 3. Manage transaction state
        
        debug!("INVITE transaction {} ready for session coordination - transaction-core handles SIP protocol", transaction_id);
        
        // Emit event for new INVITE (session-core can coordinate session creation)
        self.event_bus.publish(crate::events::SessionEvent::Custom {
            session_id: SessionId::new(), // We don't know the session yet
            event_type: "invite_ready_for_acceptance".to_string(),
            data: serde_json::json!({
                "transaction_id": transaction_id.to_string(),
            }),
        });
    }
    
    /// Handle BYE server transaction creation
    async fn handle_bye_server_transaction(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr
    ) {
        debug!("Processing BYE request - transaction-core handles SIP protocol");
        
        // Find and mark the associated dialog for termination
        if let Some(dialog_id) = self.find_dialog_for_request(&request) {
            debug!("Found dialog {} for BYE, marking for termination", dialog_id);
            if let Err(e) = self.terminate_dialog(&dialog_id).await {
                error!("Failed to terminate dialog {}: {}", dialog_id, e);
            }
        }
        
        // **ARCHITECTURAL FIX**: Don't create SIP responses here!
        // transaction-core will automatically send 200 OK for BYE
        
        // Emit call terminated event
        self.event_bus.publish(crate::events::SessionEvent::Custom {
            session_id: SessionId::new(),
            event_type: "call_terminated".to_string(),
            data: serde_json::json!({
                "transaction_id": transaction_id.to_string(),
            }),
        });
    }
    
    /// Handle other server transaction creation
    async fn handle_other_server_transaction(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr
    ) {
        debug!("Received {} request - transaction-core handles SIP protocol", request.method());
        
        // **ARCHITECTURAL FIX**: Don't create SIP responses here!
        // transaction-core will automatically send appropriate responses
        
        // Emit event for the request
        self.event_bus.publish(crate::events::SessionEvent::Custom {
            session_id: SessionId::new(), // We don't know the session yet
            event_type: format!("new_{}", request.method().to_string().to_lowercase()),
            data: serde_json::json!({
                "transaction_id": transaction_id.to_string(),
            }),
        });
    }

    /// Accept an INVITE transaction by sending 200 OK with SDP
    /// This is called by ServerManager.accept_call() to complete the call acceptance
    pub async fn accept_invite_transaction(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        sdp_answer: Option<String>
    ) -> Result<(), anyhow::Error> {
        debug!("Accepting INVITE transaction {} with SDP answer", transaction_id);
        
        // **ARCHITECTURAL FIX**: Use transaction-core's proper API
        // Instead of creating responses directly, use the transaction's send_response method
        
        // Create 200 OK response using sip-core's builder
        let mut response_builder = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            request,
            rvoip_sip_core::StatusCode::Ok,
            Some("OK")
        );
        
        // Add SDP answer if provided
        if let Some(sdp) = sdp_answer {
            response_builder = response_builder
                .content_type("application/sdp")
                .body(sdp);
        }
        
        let ok_response = response_builder.build();
        
        // Send the response through transaction-core's proper API
        if let Err(e) = self.transaction_manager.send_response(transaction_id, ok_response.clone()).await {
            return Err(anyhow::anyhow!("Failed to send 200 OK response: {}", e));
        }
        debug!("Sent 200 OK for INVITE transaction {} - call accepted!", transaction_id);
        
        // Create dialog from the INVITE transaction using the actual response we sent
        debug!("Creating dialog from accepted INVITE transaction {} with response status {}", 
               transaction_id, ok_response.status);
        if let Some(dialog_id) = self.create_dialog_from_transaction(transaction_id, request, &ok_response, false).await {
            debug!("Created dialog {} for accepted call", dialog_id);
            
            // Emit call established event
            self.event_bus.publish(crate::events::SessionEvent::Custom {
                session_id: SessionId::new(), // We don't know the session yet
                event_type: "call_established".to_string(),
                data: serde_json::json!({
                    "dialog_id": dialog_id.to_string(),
                    "transaction_id": transaction_id.to_string(),
                }),
            });
        } else {
            debug!("Failed to create dialog for INVITE transaction {}", transaction_id);
        }
        
        Ok(())
    }
} 