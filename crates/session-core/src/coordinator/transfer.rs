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
    /// Active REFER subscriptions indexed by event ID
    subscriptions: Arc<RwLock<HashMap<String, ReferSubscription>>>,
}

impl TransferHandler {
    /// Create a new transfer handler
    pub fn new(
        dialog_api: Arc<rvoip_dialog_core::api::unified::UnifiedDialogApi>,
        registry: Arc<InternalSessionRegistry>,
        dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
    ) -> Self {
        Self {
            dialog_api,
            registry,
            dialog_to_session,
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
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
        
        // Validate we have an active session for this dialog
        let session_id = self.get_session_id_for_dialog(&dialog_id).await?;
        
        // Check if this is an attended transfer (has Replaces parameter)
        if let Some(replaces_value) = replaces {
            info!("Attended transfer with Replaces: {}", replaces_value);
            // TODO: Implement attended transfer in phase 2
            // For now, return an error that dialog-core will convert to 501
            return Err(SessionError::internal("Attended transfer not yet implemented"));
        }
        
        // Create subscription for transfer progress notifications
        let event_id = self.create_refer_subscription(
            &dialog_id,
            &session_id
        ).await?;
        
        // Send initial NOTIFY (transfer pending)
        self.send_transfer_notify(
            &dialog_id,
            &event_id,
            "SIP/2.0 100 Trying\r\n",
            false, // subscription not terminated
        ).await?;
        
        // Initiate new call to transfer target
        let transfer_result = self.initiate_transfer_call(
            &session_id,
            &target_uri,
            referred_by.as_deref()
        ).await;
        
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
            .send_notify(dialog_id, "refer".to_string(), Some(notify_body))
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
        // Get original session details for caller ID
        let original_session = self.registry
            .get_session(original_session_id)
            .await?
            .ok_or_else(|| SessionError::internal("Session not found"))?;
        
        // Create metadata for the new session
        let mut metadata = HashMap::new();
        if let Some(referrer) = referred_by {
            metadata.insert("Referred-By".to_string(), referrer.to_string());
        }
        metadata.insert("Transfer-From".to_string(), original_session_id.to_string());
        
        // Create new session ID
        let new_session_id = SessionId::new();
        
        // Create new call session for the transfer
        let new_session = crate::api::types::CallSession {
            id: new_session_id.clone(),
            from: original_session.call_session.from.clone(),  // Keep original caller ID
            to: target_uri.to_string(),
            state: CallState::Initiating,
            started_at: None,
        };
        
        // Register the new session
        let internal_session = crate::session::Session::from_call_session(new_session.clone());
        self.registry.register_session(internal_session).await?;
        
        // Initiate the call through dialog API
        // Note: This creates a new outgoing INVITE
        let call_handle = self.dialog_api
            .make_call(&new_session.from, &new_session.to, None)
            .await
            .map_err(|e| SessionError::internal(
                &format!("Failed to initiate transfer call: {}", e)
            ))?;
        
        // Get the dialog ID from the call handle
        let dialog_id = call_handle.dialog().id().clone();
        
        // Map the new dialog to the new session
        self.dialog_to_session.insert(dialog_id, new_session_id.clone());
        
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
            let mut last_state = CallState::Initiating;
            let mut attempt_count = 0;
            let max_attempts = 30; // 30 seconds timeout
            
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                attempt_count += 1;
                
                // Check new call state
                if let Ok(Some(session)) = registry.get_session(&new_session_id).await {
                    let current_state = session.state().clone();
                    
                    // Only send NOTIFY if state changed
                    if current_state != last_state {
                        match current_state {
                            CallState::Ringing => {
                                // Send 180 Ringing NOTIFY
                                let _ = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 180 Ringing\r\n",
                                    false,
                                ).await;
                            }
                            CallState::Active => {
                                // Transfer succeeded - send 200 OK NOTIFY
                                let _ = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 200 OK\r\n",
                                    true, // terminate subscription
                                ).await;
                                
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
                                break;
                            }
                            CallState::Failed(reason) => {
                                // Transfer failed - send error NOTIFY
                                let _ = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 487 Request Terminated\r\n",
                                    true, // terminate subscription
                                ).await;
                                error!("Transfer failed: {}", reason);
                                
                                // Clean up subscription
                                handler.remove_subscription(&event_id).await;
                                break;
                            }
                            CallState::Terminated => {
                                // Call ended before connecting
                                let _ = handler.send_transfer_notify(
                                    &dialog_id,
                                    &event_id,
                                    "SIP/2.0 487 Request Terminated\r\n",
                                    true,
                                ).await;
                                
                                // Clean up subscription
                                handler.remove_subscription(&event_id).await;
                                break;
                            }
                            _ => {
                                // Still in progress
                            }
                        }
                        last_state = current_state;
                    }
                }
                
                if attempt_count >= max_attempts {
                    // Timeout - send error NOTIFY
                    let _ = handler.send_transfer_notify(
                        &dialog_id,
                        &event_id,
                        "SIP/2.0 408 Request Timeout\r\n",
                        true,
                    ).await;
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
            subscriptions: self.subscriptions.clone(),
        }
    }
}