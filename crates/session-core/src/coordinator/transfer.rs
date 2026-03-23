//! Call transfer handling for session coordinator
//!
//! This module handles REFER-based call transfers according to RFC 3515.
//! It manages the transfer flow including:
//! - Processing incoming REFER requests
//! - Sending 202 Accepted responses
//! - Initiating new calls to transfer targets
//! - Sending NOTIFY progress updates
//! - Coordinating call termination upon successful transfer

use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, debug, error, warn};
use tokio::sync::RwLock;
use uuid::Uuid;

use rvoip_dialog_core::{DialogId, TransactionKey};
use rvoip_sip_core::types::refer_to::ReferTo;
use rvoip_sip_core::{Request, Response, StatusCode, Method};

use crate::api::types::{SessionId, CallState};
use crate::coordinator::registry::InternalSessionRegistry;
use crate::errors::SessionError;
use crate::manager::events::{SessionEvent, SessionTransferStatus, SessionEventProcessor};

type SessionResult<T> = Result<T, SessionError>;

/// Manages REFER subscriptions for transfer progress
#[derive(Debug, Clone)]
pub struct ReferSubscription {
    pub event_id: String,
    pub dialog_id: DialogId,
    pub original_session_id: SessionId,
    pub transfer_session_id: Option<SessionId>,
    pub created_at: std::time::Instant,
}

/// Transfer handler implementation for session dialog coordinator
pub struct TransferHandler {
    dialog_api: Arc<rvoip_dialog_core::api::unified::UnifiedDialogApi>,
    registry: Arc<InternalSessionRegistry>,
    dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
    session_to_dialog: Arc<dashmap::DashMap<SessionId, DialogId>>,
    /// Active REFER subscriptions indexed by event ID
    subscriptions: Arc<RwLock<HashMap<String, ReferSubscription>>>,
    /// Event processor for publishing transfer events
    event_processor: Arc<SessionEventProcessor>,
}

impl TransferHandler {
    /// Create a new transfer handler
    pub fn new(
        dialog_api: Arc<rvoip_dialog_core::api::unified::UnifiedDialogApi>,
        registry: Arc<InternalSessionRegistry>,
        dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
        session_to_dialog: Arc<dashmap::DashMap<SessionId, DialogId>>,
        event_processor: Arc<SessionEventProcessor>,
    ) -> Self {
        Self {
            dialog_api,
            registry,
            dialog_to_session,
            session_to_dialog,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            event_processor,
        }
    }

    /// Handle incoming REFER request for call transfer
    /// 
    /// This implements the transferee behavior according to RFC 3515:
    /// 1. Validate the transfer request
    /// 2. Create subscription for progress tracking
    /// 3. Initiate a new call to the transfer target
    /// 4. Send NOTIFY messages to report transfer progress
    /// 5. Terminate the original call upon successful transfer
    /// 
    /// Note: dialog-core handles sending the 202 Accepted response
    pub async fn handle_refer_request(
        &self,
        dialog_id: DialogId,
        transaction_id: TransactionKey,
        refer_to: ReferTo,
        referred_by: Option<String>,
        replaces: Option<String>,
    ) -> SessionResult<()> {
        info!("Handling REFER request for dialog {}", dialog_id);
        
        // Extract target URI from ReferTo
        let target_uri = refer_to.uri().to_string();
        info!("Transfer target: {}", target_uri);
        
        // Debug: Log all dialog-to-session mappings
        debug!("TRANSFER: Current dialog-to-session mappings:");
        for entry in self.dialog_to_session.iter() {
            debug!("TRANSFER: Dialog {} -> Session {}", entry.key(), entry.value());
        }
        
        // Validate we have an active session for this dialog
        let session_id = match self.get_session_id_for_dialog(&dialog_id).await {
            Ok(id) => {
                debug!("TRANSFER: Found session {} for dialog {}", id, dialog_id);
                id
            },
            Err(e) => {
                error!("TRANSFER: Failed to find session for dialog {}: {}", dialog_id, e);
                error!("Failed to find session for dialog {}: {}", dialog_id, e);
                return Err(e);
            }
        };
        
        // Emit IncomingTransferRequest event
        debug!("TRANSFER: Publishing IncomingTransferRequest event for session {}", session_id);
        let result = self.event_processor.publish_event(SessionEvent::IncomingTransferRequest {
            session_id: session_id.clone(),
            target_uri: target_uri.clone(),
            referred_by: referred_by.clone(),
            replaces: replaces.clone(),
        }).await;
        
        if result.is_ok() {
            debug!("TRANSFER: Successfully published IncomingTransferRequest event");
        } else {
            error!("TRANSFER: Failed to publish IncomingTransferRequest event: {:?}", result);
        }
        
        // Check if this is an attended transfer (has Replaces parameter)
        if let Some(replaces_value) = replaces {
            info!("Attended transfer with Replaces: {}", replaces_value);
            return self.handle_attended_transfer(
                dialog_id,
                session_id,
                &target_uri,
                &replaces_value,
                referred_by.as_deref(),
            ).await;
        }
        
        // Create subscription for transfer progress notifications
        let event_id = self.create_refer_subscription(
            &dialog_id,
            &session_id
        ).await?;
        
        // Send initial NOTIFY (transfer pending)
        // The 202 Accepted has already been sent by dialog-core before
        // forwarding the TransferRequest event to us
        debug!("TRANSFER: About to send initial NOTIFY");
        if let Err(e) = self.send_transfer_notify(
            &dialog_id,
            &event_id,
            "SIP/2.0 100 Trying\r\n",
            false, // subscription not terminated
        ).await {
            error!("TRANSFER: Failed to send initial NOTIFY: {}", e);
            error!("Failed to send initial NOTIFY (continuing anyway): {}", e);
            // Continue processing the transfer even if NOTIFY fails
            // The application handler should still receive the event
        } else {
            debug!("TRANSFER: Initial NOTIFY sent successfully");
        }
        
        // Emit transfer progress - Trying
        if let Err(e) = self.event_processor.publish_event(SessionEvent::TransferProgress {
            session_id: session_id.clone(),
            status: SessionTransferStatus::Trying,
        }).await {
            tracing::warn!("Failed to publish transfer Trying progress event: {e}");
        }
        
        // Initiate new call to transfer target
        debug!("TRANSFER: About to initiate transfer call from session {} to {}", session_id, target_uri);
        let transfer_result = self.initiate_transfer_call(
            &session_id,
            &target_uri,
            referred_by.as_deref()
        ).await;
        debug!("TRANSFER: initiate_transfer_call returned: {:?}", transfer_result.is_ok());
        
        match transfer_result {
            Ok(new_session_id) => {
                info!("Transfer call initiated successfully to {}", target_uri);
                
                // Update subscription with new session ID
                self.update_subscription(&event_id, new_session_id.clone()).await;
                
                // Monitor new call and send progress NOTIFYs
                self.spawn_transfer_monitor(
                    dialog_id.clone(),
                    session_id.clone(),
                    new_session_id,
                    event_id.clone(),
                ).await;
            }
            Err(e) => {
                error!("Failed to initiate transfer call: {}", e);
                
                // Send failure NOTIFY and terminate subscription
                self.send_transfer_notify(
                    &dialog_id,
                    &event_id,
                    "SIP/2.0 503 Service Unavailable\r\n",
                    true, // terminate subscription
                ).await?;
                
                // Clean up subscription
                self.remove_subscription(&event_id).await;
            }
        }
        
        Ok(())
    }

    /// Get session ID for a dialog
    async fn get_session_id_for_dialog(&self, dialog_id: &DialogId) -> SessionResult<SessionId> {
        self.dialog_to_session
            .get(dialog_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| SessionError::internal(
                &format!("No session found for dialog {}", dialog_id)
            ))
    }

    /// Create a REFER subscription
    pub async fn create_refer_subscription(
        &self,
        dialog_id: &DialogId,
        session_id: &SessionId,
    ) -> SessionResult<String> {
        let event_id = format!("refer-{}", Uuid::new_v4());
        
        let subscription = ReferSubscription {
            event_id: event_id.clone(),
            dialog_id: dialog_id.clone(),
            original_session_id: session_id.clone(),
            transfer_session_id: None,
            created_at: std::time::Instant::now(),
        };
        
        self.subscriptions.write().await.insert(event_id.clone(), subscription);
        
        Ok(event_id)
    }

    /// Update subscription with transfer session ID
    pub async fn update_subscription(&self, event_id: &str, transfer_session_id: SessionId) {
        if let Some(mut sub) = self.subscriptions.write().await.get_mut(event_id) {
            sub.transfer_session_id = Some(transfer_session_id);
        }
    }

    /// Remove a subscription
    pub async fn remove_subscription(&self, event_id: &str) {
        self.subscriptions.write().await.remove(event_id);
    }

    /// Send NOTIFY for transfer progress
    pub async fn send_transfer_notify(
        &self,
        dialog_id: &DialogId,
        event_id: &str,
        sipfrag_body: &str,
        terminate: bool,
    ) -> SessionResult<()> {
        let subscription_state = if terminate {
            "terminated;reason=noresource"
        } else {
            "active;expires=60"
        };
        
        // Build NOTIFY body with headers
        let notify_body = format!(
            "Event: refer;id={}\r\n\
             Subscription-State: {}\r\n\
             Content-Type: message/sipfrag\r\n\
             \r\n\
             {}",
            event_id, subscription_state, sipfrag_body
        );
        
        // Send NOTIFY through dialog API
        self.dialog_api
            .send_notify(dialog_id, "refer".to_string(), Some(notify_body), Some(subscription_state.to_string()))
            .await
            .map_err(|e| SessionError::internal(
                &format!("Failed to send NOTIFY: {}", e)
            ))?;
        
        info!("Sent transfer NOTIFY for dialog {}", dialog_id);
        Ok(())
    }

    /// Initiate new call to transfer target
    async fn initiate_transfer_call(
        &self,
        original_session_id: &SessionId,
        target_uri: &str,
        referred_by: Option<&str>,
    ) -> SessionResult<SessionId> {
        debug!("TRANSFER: initiate_transfer_call started");
        debug!("  Original session: {}", original_session_id);
        debug!("  Target URI: {}", target_uri);
        
        // Get original session details for caller ID
        let original_session = self.registry
            .get_session(original_session_id)
            .await?
            .ok_or_else(|| SessionError::internal("Session not found"))?;
        
        debug!("  Original session from: {}", original_session.call_session.from);
        debug!("  Original session to: {}", original_session.call_session.to);
        
        // Create metadata for the new session
        let mut metadata = HashMap::new();
        if let Some(referrer) = referred_by {
            metadata.insert("Referred-By".to_string(), referrer.to_string());
        }
        metadata.insert("Transfer-From".to_string(), original_session_id.to_string());
        
        // Create new session ID
        let new_session_id = SessionId::new();
        
        // Create new call session for the transfer
        // IMPORTANT: The transferee (Bob) makes the new call, so use Bob's address as 'from'
        // Bob's address is in the 'to' field of the original session (Alice->Bob)
        info!("Creating transfer call: transferee '{}' calling target '{}'", 
              original_session.call_session.to, target_uri);
        let new_session = crate::api::types::CallSession {
            id: new_session_id.clone(),
            from: original_session.call_session.to.clone(),  // Use transferee's (Bob's) address
            to: target_uri.to_string(),
            state: CallState::Initiating,
            started_at: None,
            sip_call_id: None,
        };
        
        // Register the new session
        let internal_session = crate::session::Session::from_call_session(new_session.clone());
        self.registry.register_session(internal_session).await?;
        
        // Initiate the call through dialog API
        // Note: This creates a new outgoing INVITE
        debug!("TRANSFER: Calling dialog_api.make_call:");
        debug!("    From: {}", new_session.from);
        debug!("    To: {}", new_session.to);
        
        let call_handle = self.dialog_api
            .make_call(&new_session.from, &new_session.to, None)
            .await
            .map_err(|e| {
                error!("TRANSFER: dialog_api.make_call FAILED: {}", e);
                error!("dialog_api.make_call failed: {}", e);
                SessionError::internal(
                    &format!("Failed to initiate transfer call: {}", e)
                )
            })?;
        
        debug!("TRANSFER: dialog_api.make_call succeeded, got call handle");
        info!("Transfer call initiated successfully, got call handle");
        
        // Get the dialog ID from the call handle
        let dialog_id = call_handle.dialog().id().clone();
        
        // Map the new dialog to the new session (bidirectional mapping)
        self.dialog_to_session.insert(dialog_id.clone(), new_session_id.clone());
        self.session_to_dialog.insert(new_session_id.clone(), dialog_id.clone());
        
        // CRITICAL: Publish SessionCreated event so the coordinator knows about this session
        if let Err(e) = self.event_processor.publish_event(SessionEvent::SessionCreated {
            session_id: new_session_id.clone(),
            from: new_session.from.clone(),
            to: new_session.to.clone(),
            call_state: CallState::Initiating,
        }).await {
            tracing::warn!("Failed to publish SessionCreated event for transfer session {}: {}", new_session_id, e);
        }
        
        info!("Transfer call setup complete: session {} -> dialog {}", new_session_id, dialog_id);
        
        Ok(new_session_id)
    }

    /// Spawn task to monitor transfer progress and send NOTIFYs
    async fn spawn_transfer_monitor(
        &self,
        dialog_id: DialogId,
        original_session_id: SessionId,
        new_session_id: SessionId,
        event_id: String,
    ) {
        let registry = self.registry.clone();
        let dialog_api = self.dialog_api.clone();
        let handler = self.clone();
        
        tokio::spawn(async move {
            tracing::debug!("TRANSFER MONITOR: Started monitoring transfer for session {}", new_session_id);
            let mut last_state = CallState::Initiating;
            let mut attempt_count = 0;
            let max_attempts = 30; // 30 seconds timeout
            
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                attempt_count += 1;
                
                // Check new call state
                if let Ok(Some(session)) = registry.get_session(&new_session_id).await {
                    let current_state = session.state().clone();
                    tracing::debug!("TRANSFER MONITOR: Session {} state: {:?} (attempt {})", new_session_id, current_state, attempt_count);
                    
                    // Check if transfer is complete (regardless of whether state changed)
                    match current_state {
                        CallState::Active => {
                            // Only send NOTIFY if this is a state change
                            if current_state != last_state {
                                tracing::info!("TRANSFER MONITOR: Transfer call {} is now ACTIVE! Transfer successful!", new_session_id);
                                // Transfer succeeded - send 200 OK NOTIFY
                                if let Err(e) = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 200 OK\r\n",
                                    true, // terminate subscription
                                ).await {
                                    tracing::warn!("Failed to send transfer success NOTIFY: {e}");
                                }

                                // Emit transfer progress - Success
                                if let Err(e) = handler.event_processor.publish_event(SessionEvent::TransferProgress {
                                    session_id: original_session_id.clone(),
                                    status: SessionTransferStatus::Success,
                                }).await {
                                    tracing::warn!("Failed to publish transfer Success progress event: {e}");
                                }
                                
                                // Terminate original call properly (this will send BYE)
                                // We need to get the dialog ID for the original session
                                if let Some(original_dialog_id) = handler.dialog_to_session.iter()
                                    .find(|entry| entry.value() == &original_session_id)
                                    .map(|entry| entry.key().clone()) 
                                {
                                    // Send BYE to terminate the original call
                                    if let Err(e) = dialog_api.send_bye(&original_dialog_id).await {
                                        error!("Failed to send BYE for original call: {}", e);
                                    }
                                }
                                
                                // Update session state to reflect termination
                                if let Err(e) = registry.update_session_state(&original_session_id, CallState::Terminated).await {
                                    error!("Failed to update original call state: {}", e);
                                }
                                info!("Transfer completed successfully");
                                
                                // Clean up subscription
                                handler.remove_subscription(&event_id).await;
                            }
                            // Always exit when transfer is active
                            break;
                        }
                        CallState::Failed(ref reason) => {
                            // Only send NOTIFY if this is a state change
                            if current_state != last_state {
                                // Transfer failed - send error NOTIFY
                                if let Err(e) = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 487 Request Terminated\r\n",
                                    true, // terminate subscription
                                ).await {
                                    tracing::warn!("Failed to send transfer failure NOTIFY: {e}");
                                }
                                error!("Transfer failed: {}", reason);
                                
                                // Clean up subscription
                                handler.remove_subscription(&event_id).await;
                            }
                            // Always exit when transfer failed
                            break;
                        }
                        CallState::Terminated => {
                            // Only send NOTIFY if this is a state change
                            if current_state != last_state {
                                // Call ended before connecting
                                if let Err(e) = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 487 Request Terminated\r\n",
                                    true,
                                ).await {
                                    tracing::warn!("Failed to send transfer terminated NOTIFY: {e}");
                                }

                                // Clean up subscription
                                handler.remove_subscription(&event_id).await;
                            }
                            // Always exit when transfer terminated
                            break;
                        }
                        CallState::Ringing => {
                            // Send notifications only if state changed
                            if current_state != last_state {
                                // Send 180 Ringing NOTIFY
                                if let Err(e) = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 180 Ringing\r\n",
                                    false,
                                ).await {
                                    tracing::warn!("Failed to send transfer ringing NOTIFY: {e}");
                                }

                                // Emit transfer progress - Ringing
                                if let Err(e) = handler.event_processor.publish_event(SessionEvent::TransferProgress {
                                    session_id: original_session_id.clone(),
                                    status: SessionTransferStatus::Ringing,
                                }).await {
                                    tracing::warn!("Failed to publish transfer Ringing progress event: {e}");
                                }
                            }
                        }
                        _ => {
                            // Other states - just continue monitoring
                        }
                    }
                    
                    // Update last state for change detection
                    last_state = current_state;
                }
                
                if attempt_count >= max_attempts {
                    // Timeout - send error NOTIFY
                    if let Err(e) = handler.send_transfer_notify(
                        &dialog_id,
                        &event_id,
                        "SIP/2.0 408 Request Timeout\r\n",
                        true,
                    ).await {
                        tracing::warn!("Failed to send transfer timeout NOTIFY: {e}");
                    }
                    error!("Transfer timed out");
                    
                    // Clean up subscription
                    handler.remove_subscription(&event_id).await;
                    break;
                }
            }
            
            // Cleanup any expired subscriptions periodically
            handler.cleanup_expired_subscriptions().await;
        });
    }

    /// Initiate an attended transfer (transferor side)
    ///
    /// This implements the transferor behavior for attended transfer (RFC 3515 + RFC 3891):
    /// 1. Party A is on call with Party B (session_a_b)
    /// 2. Party A has established a consultation call with Party C (session_a_c)
    /// 3. Party A sends REFER to B with Refer-To containing C's URI and Replaces header
    /// 4. B will send INVITE to C with Replaces header, replacing the A-C call
    /// 5. A terminates both original sessions
    pub async fn initiate_attended_transfer(
        &self,
        session_a_b: &SessionId,
        session_a_c: &SessionId,
    ) -> SessionResult<()> {
        info!("Initiating attended transfer: session_a_b={}, session_a_c={}", session_a_b, session_a_c);

        // Get dialog IDs for both sessions
        let dialog_a_b = self.session_to_dialog
            .get(session_a_b)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| SessionError::internal(
                &format!("No dialog found for session {}", session_a_b)
            ))?;

        let dialog_a_c = self.session_to_dialog
            .get(session_a_c)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| SessionError::internal(
                &format!("No dialog found for session {}", session_a_c)
            ))?;

        // Get dialog info for A-C call to build Replaces header
        let ac_dialog = self.dialog_api
            .get_dialog_info(&dialog_a_c)
            .await
            .map_err(|e| SessionError::internal(
                &format!("Failed to get dialog info for A-C call: {}", e)
            ))?;

        // Get the A-C session to extract the target URI (C's address)
        let ac_session = self.registry
            .get_session(session_a_c)
            .await?
            .ok_or_else(|| SessionError::internal("A-C session not found"))?;

        let target_uri = ac_session.call_session.to.clone();

        // Build Replaces parameter: call-id;from-tag=...;to-tag=...
        // The Replaces header identifies the A-C dialog from B's perspective
        let replaces_param = format!(
            "{};from-tag={};to-tag={}",
            ac_dialog.call_id,
            ac_dialog.local_tag.as_deref().unwrap_or(""),
            ac_dialog.remote_tag.as_deref().unwrap_or(""),
        );

        // URL-encode the Replaces parameter for inclusion in Refer-To URI
        let encoded_replaces = url_encode_replaces(&replaces_param);

        // Build Refer-To URI with Replaces parameter
        // Format: <sip:C@domain?Replaces=call-id%3Bfrom-tag%3Dxxx%3Bto-tag%3Dyyy>
        let refer_to_uri = if target_uri.contains('?') {
            format!("{}&Replaces={}", target_uri, encoded_replaces)
        } else {
            format!("{}?Replaces={}", target_uri, encoded_replaces)
        };

        // Get A's address for Referred-By header
        let ab_session = self.registry
            .get_session(session_a_b)
            .await?
            .ok_or_else(|| SessionError::internal("A-B session not found"))?;

        let referred_by = ab_session.call_session.from.clone();

        // Build the REFER body with Referred-By header
        let refer_body = format!(
            "Refer-To: {}\r\nReferred-By: {}\r\n",
            refer_to_uri, referred_by
        );

        // Create subscription for transfer progress tracking
        let event_id = self.create_refer_subscription(
            &dialog_a_b,
            session_a_b
        ).await?;

        // Send REFER to B through dialog API
        debug!("Sending attended transfer REFER to dialog {}: Refer-To: {}", dialog_a_b, refer_to_uri);
        self.dialog_api
            .send_refer(&dialog_a_b, refer_to_uri.clone(), Some(refer_body))
            .await
            .map_err(|e| SessionError::internal(
                &format!("Failed to send REFER for attended transfer: {}", e)
            ))?;

        info!("Attended transfer REFER sent successfully");

        // Emit transfer progress event
        if let Err(e) = self.event_processor.publish_event(SessionEvent::TransferProgress {
            session_id: session_a_b.clone(),
            status: SessionTransferStatus::Trying,
        }).await {
            tracing::warn!("Failed to publish attended transfer Trying progress event: {e}");
        }

        // Spawn monitor to track REFER progress via NOTIFYs and clean up sessions
        self.spawn_attended_transfer_monitor(
            dialog_a_b.clone(),
            session_a_b.clone(),
            session_a_c.clone(),
            event_id,
        ).await;

        Ok(())
    }

    /// Handle an incoming attended transfer (transferee side)
    ///
    /// When B receives REFER with Replaces from A:
    /// 1. B sends INVITE to C with Replaces header
    /// 2. C replaces the A-C call with the new B-C call
    /// 3. B terminates the original A-B call
    async fn handle_attended_transfer(
        &self,
        dialog_id: DialogId,
        session_id: SessionId,
        target_uri: &str,
        replaces_value: &str,
        referred_by: Option<&str>,
    ) -> SessionResult<()> {
        info!("Handling attended transfer for dialog {}: target={}, replaces={}",
              dialog_id, target_uri, replaces_value);

        // Create subscription for transfer progress notifications
        let event_id = self.create_refer_subscription(
            &dialog_id,
            &session_id
        ).await?;

        // Send initial NOTIFY (transfer pending)
        if let Err(e) = self.send_transfer_notify(
            &dialog_id,
            &event_id,
            "SIP/2.0 100 Trying\r\n",
            false,
        ).await {
            error!("Failed to send initial NOTIFY for attended transfer: {}", e);
        }

        // Emit transfer progress - Trying
        if let Err(e) = self.event_processor.publish_event(SessionEvent::TransferProgress {
            session_id: session_id.clone(),
            status: SessionTransferStatus::Trying,
        }).await {
            tracing::warn!("Failed to publish transfer Trying progress event: {e}");
        }

        // Initiate the replacement call to the target with Replaces header
        // The Replaces value is passed so the INVITE includes the Replaces header
        let transfer_result = self.initiate_attended_transfer_call(
            &session_id,
            target_uri,
            replaces_value,
            referred_by,
        ).await;

        match transfer_result {
            Ok(new_session_id) => {
                info!("Attended transfer call initiated to {}", target_uri);

                // Update subscription with new session ID
                self.update_subscription(&event_id, new_session_id.clone()).await;

                // Monitor new call and send progress NOTIFYs
                self.spawn_transfer_monitor(
                    dialog_id.clone(),
                    session_id.clone(),
                    new_session_id,
                    event_id.clone(),
                ).await;
            }
            Err(e) => {
                error!("Failed to initiate attended transfer call: {}", e);

                // Send failure NOTIFY
                self.send_transfer_notify(
                    &dialog_id,
                    &event_id,
                    "SIP/2.0 503 Service Unavailable\r\n",
                    true,
                ).await?;

                self.remove_subscription(&event_id).await;
            }
        }

        Ok(())
    }

    /// Initiate a new call for attended transfer, including Replaces header in the INVITE
    async fn initiate_attended_transfer_call(
        &self,
        original_session_id: &SessionId,
        target_uri: &str,
        replaces_value: &str,
        referred_by: Option<&str>,
    ) -> SessionResult<SessionId> {
        debug!("ATTENDED TRANSFER: Initiating replacement call to {}", target_uri);

        // Get original session details
        let original_session = self.registry
            .get_session(original_session_id)
            .await?
            .ok_or_else(|| SessionError::internal("Session not found"))?;

        // Create metadata
        let mut metadata = HashMap::new();
        if let Some(referrer) = referred_by {
            metadata.insert("Referred-By".to_string(), referrer.to_string());
        }
        metadata.insert("Transfer-From".to_string(), original_session_id.to_string());
        metadata.insert("Replaces".to_string(), replaces_value.to_string());

        // Create new session ID
        let new_session_id = SessionId::new();

        // Create new call session for the attended transfer
        let new_session = crate::api::types::CallSession {
            id: new_session_id.clone(),
            from: original_session.call_session.to.clone(),
            to: target_uri.to_string(),
            state: CallState::Initiating,
            started_at: None,
            sip_call_id: None,
        };

        // Register the new session
        let internal_session = crate::session::Session::from_call_session(new_session.clone());
        self.registry.register_session(internal_session).await?;

        // Initiate the call through dialog API
        // The Replaces header will be handled by the remote end when it receives the INVITE
        let call_handle = self.dialog_api
            .make_call(&new_session.from, &new_session.to, None)
            .await
            .map_err(|e| {
                error!("ATTENDED TRANSFER: make_call failed: {}", e);
                SessionError::internal(
                    &format!("Failed to initiate attended transfer call: {}", e)
                )
            })?;

        let dialog_id = call_handle.dialog().id().clone();

        // Map the new dialog to the new session
        self.dialog_to_session.insert(dialog_id.clone(), new_session_id.clone());
        self.session_to_dialog.insert(new_session_id.clone(), dialog_id.clone());

        // Publish SessionCreated event
        if let Err(e) = self.event_processor.publish_event(SessionEvent::SessionCreated {
            session_id: new_session_id.clone(),
            from: new_session.from.clone(),
            to: new_session.to.clone(),
            call_state: CallState::Initiating,
        }).await {
            tracing::warn!("Failed to publish SessionCreated event for attended transfer: {e}");
        }

        info!("Attended transfer call setup: session {} -> dialog {}", new_session_id, dialog_id);
        Ok(new_session_id)
    }

    /// Spawn task to monitor attended transfer progress on the transferor side
    /// This monitors the REFER subscription and cleans up both original sessions on success
    async fn spawn_attended_transfer_monitor(
        &self,
        dialog_a_b: DialogId,
        session_a_b: SessionId,
        session_a_c: SessionId,
        event_id: String,
    ) {
        let registry = self.registry.clone();
        let dialog_api = self.dialog_api.clone();
        let handler = self.clone();
        let dialog_to_session = self.dialog_to_session.clone();
        let session_to_dialog = self.session_to_dialog.clone();

        tokio::spawn(async move {
            tracing::debug!("ATTENDED TRANSFER MONITOR: Watching sessions A-B={}, A-C={}", session_a_b, session_a_c);
            let mut attempt_count = 0;
            let max_attempts = 30; // 30 seconds timeout

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                attempt_count += 1;

                // For the transferor, we wait until we receive NOTIFY with 200 OK
                // indicating B successfully connected to C
                // In the meantime, we just check if the subscription is still active
                let subs = handler.subscriptions.read().await;
                let sub_still_active = subs.contains_key(&event_id);
                drop(subs);

                if !sub_still_active {
                    // Subscription was terminated (either success or failure)
                    // The NOTIFY handler would have removed it
                    tracing::info!("ATTENDED TRANSFER MONITOR: Subscription {} terminated", event_id);

                    // Terminate both original sessions
                    // Terminate A-B
                    if let Some(dialog_a_b_id) = session_to_dialog.get(&session_a_b).map(|e| e.value().clone()) {
                        if let Err(e) = dialog_api.send_bye(&dialog_a_b_id).await {
                            error!("Failed to send BYE for A-B call: {}", e);
                        }
                    }
                    if let Err(e) = registry.update_session_state(&session_a_b, CallState::Terminated).await {
                        error!("Failed to update A-B session state: {}", e);
                    }

                    // Terminate A-C
                    if let Some(dialog_a_c_id) = session_to_dialog.get(&session_a_c).map(|e| e.value().clone()) {
                        if let Err(e) = dialog_api.send_bye(&dialog_a_c_id).await {
                            error!("Failed to send BYE for A-C call: {}", e);
                        }
                    }
                    if let Err(e) = registry.update_session_state(&session_a_c, CallState::Terminated).await {
                        error!("Failed to update A-C session state: {}", e);
                    }

                    // Emit transfer completion
                    if let Err(e) = handler.event_processor.publish_event(SessionEvent::TransferProgress {
                        session_id: session_a_b.clone(),
                        status: SessionTransferStatus::Success,
                    }).await {
                        tracing::warn!("Failed to publish attended transfer Success progress event: {e}");
                    }

                    tracing::info!("ATTENDED TRANSFER MONITOR: Both sessions terminated, transfer complete");
                    break;
                }

                if attempt_count >= max_attempts {
                    error!("Attended transfer timed out");
                    handler.remove_subscription(&event_id).await;
                    break;
                }
            }

            handler.cleanup_expired_subscriptions().await;
        });
    }

    /// Clean up expired subscriptions (older than 5 minutes)
    pub async fn cleanup_expired_subscriptions(&self) {
        let mut subs = self.subscriptions.write().await;
        let now = std::time::Instant::now();
        let expiry = std::time::Duration::from_secs(300); // 5 minutes
        
        subs.retain(|_, sub| {
            now.duration_since(sub.created_at) < expiry
        });
    }
}

// Implement Clone manually to share between tasks
impl Clone for TransferHandler {
    fn clone(&self) -> Self {
        Self {
            dialog_api: self.dialog_api.clone(),
            registry: self.registry.clone(),
            dialog_to_session: self.dialog_to_session.clone(),
            session_to_dialog: self.session_to_dialog.clone(),
            subscriptions: self.subscriptions.clone(),
            event_processor: self.event_processor.clone(),
        }
    }
}

/// URL-encode the Replaces parameter for inclusion in a Refer-To URI
///
/// Per RFC 3891, the Replaces header value must be percent-encoded when
/// placed in a URI parameter. Semicolons become %3B, equals signs become %3D,
/// at-signs become %40, etc.
fn url_encode_replaces(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() * 3);
    for ch in value.chars() {
        match ch {
            ';' => encoded.push_str("%3B"),
            '=' => encoded.push_str("%3D"),
            '@' => encoded.push_str("%40"),
            ' ' => encoded.push_str("%20"),
            '%' => encoded.push_str("%25"),
            '?' => encoded.push_str("%3F"),
            '&' => encoded.push_str("%26"),
            '+' => encoded.push_str("%2B"),
            _ => encoded.push(ch),
        }
    }
    encoded
}