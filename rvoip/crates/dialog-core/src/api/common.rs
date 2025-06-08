//! Common API Types
//!
//! This module provides shared types and handles used across the dialog-core API,
//! offering convenient access to dialog operations and events.
//!
//! ## Overview
//!
//! The common types provide high-level abstractions over SIP dialogs and calls:
//!
//! - **DialogHandle**: General-purpose dialog operations and state management
//! - **CallHandle**: Call-specific operations built on top of DialogHandle
//! - **CallInfo**: Information about active calls
//! - **DialogEvent**: Events that can be monitored by applications
//!
//! ## Quick Start
//!
//! ### Using DialogHandle
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::common::DialogHandle;
//! use rvoip_sip_core::Method;
//!
//! # async fn example(dialog: DialogHandle) -> Result<(), Box<dyn std::error::Error>> {
//! // Send a request within the dialog
//! let tx_key = dialog.send_request(Method::Info, Some("Hello".to_string())).await?;
//! println!("Sent INFO request: {}", tx_key);
//!
//! // Check dialog state
//! let state = dialog.state().await?;
//! println!("Dialog state: {:?}", state);
//!
//! // Send BYE to terminate
//! dialog.send_bye().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Using CallHandle
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::common::CallHandle;
//!
//! # async fn example(call: CallHandle) -> Result<(), Box<dyn std::error::Error>> {
//! // Get call information
//! let info = call.info().await?;
//! println!("Call from {} to {}", info.local_uri, info.remote_uri);
//!
//! // Put call on hold
//! call.hold(Some("SDP with hold attributes".to_string())).await?;
//!
//! // Transfer the call
//! call.transfer("sip:voicemail@example.com".to_string()).await?;
//!
//! // Hang up
//! call.hangup().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Handle Architecture
//!
//! ```text
//! CallHandle
//!     │
//!     └── DialogHandle
//!             │
//!             └── DialogManager
//!                     │
//!                     └── Dialog (actual state)
//! ```
//!
//! CallHandle provides call-specific operations while DialogHandle provides
//! general dialog operations. Both are lightweight wrappers that delegate
//! to the underlying DialogManager.
//!
//! ## Event Monitoring
//!
//! Applications can monitor dialog events for state changes:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::common::DialogEvent;
//!
//! fn handle_dialog_event(event: DialogEvent) {
//!     match event {
//!         DialogEvent::Created { dialog_id } => {
//!             println!("New dialog: {}", dialog_id);
//!         },
//!         DialogEvent::StateChanged { dialog_id, old_state, new_state } => {
//!             println!("Dialog {} changed from {:?} to {:?}", dialog_id, old_state, new_state);
//!         },
//!         DialogEvent::Terminated { dialog_id, reason } => {
//!             println!("Dialog {} terminated: {}", dialog_id, reason);
//!         },
//!         _ => {}
//!     }
//! }
//! ```

use std::sync::Arc;
use tracing::{debug, info, warn};

use rvoip_sip_core::{Method, StatusCode, Response};
use rvoip_transaction_core::TransactionKey;
use crate::manager::DialogManager;
use crate::dialog::{DialogId, Dialog, DialogState};
use super::{ApiResult, ApiError};

/// A handle to a SIP dialog for convenient operations
/// 
/// Provides a high-level interface to dialog operations without exposing
/// the underlying DialogManager complexity. DialogHandle is the primary
/// way applications interact with SIP dialogs, offering both basic and
/// advanced dialog management capabilities.
///
/// ## Key Features
///
/// - **State Management**: Query and monitor dialog state changes
/// - **Request Sending**: Send arbitrary SIP methods within the dialog
/// - **Response Handling**: Send responses to incoming requests
/// - **Lifecycle Control**: Terminate dialogs gracefully or immediately
/// - **SIP Methods**: Convenient methods for common SIP operations
///
/// ## Examples
///
/// ### Basic Dialog Operations
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::DialogHandle;
/// use rvoip_sip_core::Method;
///
/// # async fn example(dialog: DialogHandle) -> Result<(), Box<dyn std::error::Error>> {
/// // Check if dialog is still active
/// if dialog.is_active().await {
///     println!("Dialog {} is active", dialog.id());
///     
///     // Get current state
///     let state = dialog.state().await?;
///     println!("State: {:?}", state);
///     
///     // Send a custom request
///     let tx_key = dialog.send_request(Method::Update, Some("new parameters".to_string())).await?;
///     println!("Sent UPDATE: {}", tx_key);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Advanced Dialog Usage
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::DialogHandle;
/// use rvoip_sip_core::Method;
///
/// # async fn example(dialog: DialogHandle) -> Result<(), Box<dyn std::error::Error>> {
/// // Get full dialog information
/// let info = dialog.info().await?;
/// println!("Dialog: {} -> {}", info.local_uri, info.remote_uri);
/// println!("Call-ID: {}, State: {:?}", info.call_id, info.state);
///
/// // Send application-specific information
/// dialog.send_info("Custom application data".to_string()).await?;
///
/// // Send a notification about an event
/// dialog.send_notify("presence".to_string(), Some("online".to_string())).await?;
///
/// // Gracefully terminate the dialog
/// dialog.terminate().await?;
/// # Ok(())
/// # }
/// ```
///
/// ### Dialog State Monitoring
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::DialogHandle;
/// use rvoip_dialog_core::dialog::DialogState;
///
/// # async fn example(dialog: DialogHandle) -> Result<(), Box<dyn std::error::Error>> {
/// // Monitor dialog state changes
/// loop {
///     let state = dialog.state().await?;
///     match state {
///         DialogState::Initial => println!("Dialog starting..."),
///         DialogState::Early => println!("Dialog in early state"),
///         DialogState::Confirmed => {
///             println!("Dialog confirmed - ready for operations");
///             break;
///         },
///         DialogState::Terminated => {
///             println!("Dialog terminated");
///             break;
///         },
///         DialogState::Recovering => {
///             println!("Dialog recovering...");
///         },
///     }
///     tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DialogHandle {
    dialog_id: DialogId,
    dialog_manager: Arc<DialogManager>,
}

impl DialogHandle {
    /// Create a new dialog handle
    pub(crate) fn new(dialog_id: DialogId, dialog_manager: Arc<DialogManager>) -> Self {
        Self {
            dialog_id,
            dialog_manager,
        }
    }
    
    /// Get the dialog ID
    pub fn id(&self) -> &DialogId {
        &self.dialog_id
    }
    
    /// Get the current dialog information
    pub async fn info(&self) -> ApiResult<Dialog> {
        self.dialog_manager.get_dialog(&self.dialog_id)
            .map_err(ApiError::from)
    }
    
    /// Get the current dialog state
    pub async fn state(&self) -> ApiResult<DialogState> {
        self.dialog_manager.get_dialog_state(&self.dialog_id)
            .map_err(ApiError::from)
    }
    
    /// Send a request within this dialog
    /// 
    /// # Arguments
    /// * `method` - SIP method to send
    /// * `body` - Optional message body
    /// 
    /// # Returns
    /// Transaction key for tracking the request
    pub async fn send_request(&self, method: Method, body: Option<String>) -> ApiResult<String> {
        debug!("Sending {} request in dialog {}", method, self.dialog_id);
        
        let body_bytes = body.map(|s| bytes::Bytes::from(s));
        let transaction_key = self.dialog_manager.send_request(&self.dialog_id, method, body_bytes).await
            .map_err(ApiError::from)?;
        
        Ok(transaction_key.to_string())
    }
    
    /// **NEW**: Send a request within this dialog (returns TransactionKey)
    /// 
    /// Enhanced version that returns the actual TransactionKey for advanced usage.
    /// 
    /// # Arguments
    /// * `method` - SIP method to send
    /// * `body` - Optional message body
    /// 
    /// # Returns
    /// Transaction key for tracking the request
    pub async fn send_request_with_key(&self, method: Method, body: Option<bytes::Bytes>) -> ApiResult<TransactionKey> {
        debug!("Sending {} request in dialog {}", method, self.dialog_id);
        
        self.dialog_manager.send_request(&self.dialog_id, method, body).await
            .map_err(ApiError::from)
    }
    
    /// **NEW**: Send a SIP response for a transaction
    /// 
    /// Allows sending responses directly through the dialog handle.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `response` - Complete SIP response
    /// 
    /// # Returns
    /// Success or error
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> ApiResult<()> {
        debug!("Sending response for transaction {} in dialog {}", transaction_id, self.dialog_id);
        
        self.dialog_manager.send_response(transaction_id, response).await
            .map_err(ApiError::from)
    }
    
    /// **NEW**: Send specific SIP methods with convenience
    
    /// Send a BYE request to terminate the dialog
    pub async fn send_bye(&self) -> ApiResult<TransactionKey> {
        info!("Sending BYE for dialog {}", self.dialog_id);
        self.send_request_with_key(Method::Bye, None).await
    }
    
    /// Send a REFER request for call transfer
    pub async fn send_refer(&self, target_uri: String, refer_body: Option<String>) -> ApiResult<TransactionKey> {
        info!("Sending REFER for dialog {} to {}", self.dialog_id, target_uri);
        
        let body = if let Some(custom_body) = refer_body {
            custom_body
        } else {
            format!("Refer-To: {}\r\n", target_uri)
        };
        
        self.send_request_with_key(Method::Refer, Some(bytes::Bytes::from(body))).await
    }
    
    /// Send a NOTIFY request for event notifications
    pub async fn send_notify(&self, event: String, body: Option<String>) -> ApiResult<TransactionKey> {
        info!("Sending NOTIFY for dialog {} event {}", self.dialog_id, event);
        
        let notify_body = body.map(|b| bytes::Bytes::from(b));
        self.send_request_with_key(Method::Notify, notify_body).await
    }
    
    /// Send an UPDATE request for media modifications
    pub async fn send_update(&self, sdp: Option<String>) -> ApiResult<TransactionKey> {
        info!("Sending UPDATE for dialog {}", self.dialog_id);
        
        let update_body = sdp.map(|s| bytes::Bytes::from(s));
        self.send_request_with_key(Method::Update, update_body).await
    }
    
    /// Send an INFO request for application-specific information
    pub async fn send_info(&self, info_body: String) -> ApiResult<TransactionKey> {
        info!("Sending INFO for dialog {}", self.dialog_id);
        
        self.send_request_with_key(Method::Info, Some(bytes::Bytes::from(info_body))).await
    }
    
    /// Send BYE to terminate the dialog
    pub async fn terminate(&self) -> ApiResult<()> {
        info!("Terminating dialog {}", self.dialog_id);
        
        // Send BYE request
        self.send_request(Method::Bye, None).await?;
        
        // Terminate dialog
        self.dialog_manager.terminate_dialog(&self.dialog_id).await
            .map_err(ApiError::from)?;
        
        Ok(())
    }
    
    /// **NEW**: Terminate dialog directly without sending BYE
    /// 
    /// For cases where you want to clean up the dialog state without
    /// sending a BYE request (e.g., after receiving a BYE).
    pub async fn terminate_immediately(&self) -> ApiResult<()> {
        info!("Terminating dialog {} immediately", self.dialog_id);
        
        self.dialog_manager.terminate_dialog(&self.dialog_id).await
            .map_err(ApiError::from)
    }
    
    /// Check if the dialog is still active
    pub async fn is_active(&self) -> bool {
        self.dialog_manager.has_dialog(&self.dialog_id)
    }
}

/// A handle to a SIP call (specific type of dialog) for call-related operations
/// 
/// Provides call-specific convenience methods on top of the basic dialog operations.
/// CallHandle extends DialogHandle with operations that are specifically relevant
/// to voice/video calls, such as hold/resume, transfer, and media management.
///
/// ## Key Features
///
/// - **Call Lifecycle**: Answer, reject, and hang up calls
/// - **Call Control**: Hold, resume, transfer, and mute operations
/// - **Media Management**: Update media parameters and handle media events
/// - **Call Information**: Access to call-specific metadata and state
/// - **Dialog Access**: Full access to underlying dialog operations
///
/// ## Examples
///
/// ### Basic Call Operations
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::CallHandle;
/// use rvoip_sip_core::StatusCode;
///
/// # async fn example(call: CallHandle) -> Result<(), Box<dyn std::error::Error>> {
/// // Get call information
/// let info = call.info().await?;
/// println!("Call {} from {} to {}", info.call_id, info.local_uri, info.remote_uri);
/// println!("State: {:?}", info.state);
///
/// // Answer an incoming call
/// call.answer(Some("SDP answer with media info".to_string())).await?;
///
/// // Or reject a call
/// // call.reject(StatusCode::Busy, Some("Busy right now".to_string())).await?;
/// # Ok(())
/// # }
/// ```
///
/// ### Call Control Operations
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::CallHandle;
///
/// # async fn example(call: CallHandle) -> Result<(), Box<dyn std::error::Error>> {
/// // Put call on hold
/// let hold_sdp = "v=0\r\no=- 123 456 IN IP4 0.0.0.0\r\n...";
/// call.hold(Some(hold_sdp.to_string())).await?;
/// println!("Call on hold");
///
/// // Resume from hold
/// let resume_sdp = "v=0\r\no=- 123 457 IN IP4 192.168.1.100\r\n...";
/// call.resume(Some(resume_sdp.to_string())).await?;
/// println!("Call resumed");
///
/// // Transfer to another party
/// call.transfer("sip:alice@example.com".to_string()).await?;
/// println!("Call transferred");
/// # Ok(())
/// # }
/// ```
///
/// ### Advanced Call Management
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::CallHandle;
///
/// # async fn example(call: CallHandle) -> Result<(), Box<dyn std::error::Error>> {
/// // Update media parameters
/// let new_sdp = "v=0\r\no=- 123 458 IN IP4 192.168.1.100\r\n...";
/// call.update_media(Some(new_sdp.to_string())).await?;
///
/// // Send call-related information
/// call.send_info("DTMF: 1".to_string()).await?;
///
/// // Send call event notification
/// call.notify("call-status".to_string(), Some("ringing".to_string())).await?;
///
/// // Advanced transfer with custom REFER
/// let refer_body = "Refer-To: sip:bob@example.com\r\nReplaces: abc123";
/// call.transfer_with_body("sip:bob@example.com".to_string(), refer_body.to_string()).await?;
/// # Ok(())
/// # }
/// ```
///
/// ### Call State Monitoring
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::CallHandle;
/// use rvoip_dialog_core::dialog::DialogState;
///
/// # async fn example(call: CallHandle) -> Result<(), Box<dyn std::error::Error>> {
/// // Monitor call state
/// while call.is_active().await {
///     let state = call.dialog_state().await?;
///     match state {
///         DialogState::Early => println!("Call ringing..."),
///         DialogState::Confirmed => {
///             println!("Call answered and active");
///             break;
///         },
///         DialogState::Terminated => {
///             println!("Call ended");
///             break;
///         },
///         _ => {}
///     }
///     tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Integration with Dialog Operations
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::CallHandle;
/// use rvoip_sip_core::Method;
///
/// # async fn example(call: CallHandle) -> Result<(), Box<dyn std::error::Error>> {
/// // Access underlying dialog handle
/// let dialog = call.dialog();
/// 
/// // Use dialog operations directly
/// let tx_key = dialog.send_request(Method::Options, None).await?;
/// println!("Sent OPTIONS via dialog: {}", tx_key);
///
/// // Or use call-specific shortcuts
/// let tx_key = call.send_request(Method::Info, Some("Call info".to_string())).await?;
/// println!("Sent INFO via call: {}", tx_key);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CallHandle {
    dialog_handle: DialogHandle,
}

impl CallHandle {
    /// Create a new call handle
    pub(crate) fn new(dialog_id: DialogId, dialog_manager: Arc<DialogManager>) -> Self {
        Self {
            dialog_handle: DialogHandle::new(dialog_id, dialog_manager),
        }
    }
    
    /// Get the underlying dialog handle
    pub fn dialog(&self) -> &DialogHandle {
        &self.dialog_handle
    }
    
    /// Get the call ID (same as dialog ID)
    pub fn call_id(&self) -> &DialogId {
        self.dialog_handle.id()
    }
    
    /// Get call information
    pub async fn info(&self) -> ApiResult<CallInfo> {
        let dialog = self.dialog_handle.info().await?;
        Ok(CallInfo {
            call_id: dialog.id.clone(),
            state: dialog.state,
            local_uri: dialog.local_uri.to_string(),
            remote_uri: dialog.remote_uri.to_string(),
            call_id_header: dialog.call_id,
            local_tag: dialog.local_tag,
            remote_tag: dialog.remote_tag,
        })
    }
    
    /// Answer the call (send 200 OK)
    /// 
    /// # Arguments
    /// * `sdp_answer` - Optional SDP answer for media negotiation
    /// 
    /// # Returns
    /// Success or error
    pub async fn answer(&self, sdp_answer: Option<String>) -> ApiResult<()> {
        info!("Answering call {}", self.call_id());
        
        // Find the transaction associated with this dialog
        // We need to look through the transaction-to-dialog mappings to find the INVITE transaction
        let transaction_id = {
            let dialog_manager = &self.dialog_handle.dialog_manager;
            let mut found_tx_id = None;
            
            // Search through transaction mappings to find the INVITE transaction for this dialog
            for entry in dialog_manager.transaction_to_dialog.iter() {
                if entry.value() == self.call_id() {
                    // Check if this is an INVITE transaction (server-side)
                    let tx_key = entry.key();
                    if tx_key.to_string().contains("INVITE") && tx_key.to_string().contains("server") {
                        found_tx_id = Some(tx_key.clone());
                        break;
                    }
                }
            }
            
            found_tx_id.ok_or_else(|| ApiError::Internal {
                message: "No INVITE transaction found for this call".to_string()
            })?
        };
        
        // Get the original INVITE request to build a proper response
        let original_request = self.dialog_handle.dialog_manager
            .transaction_manager()
            .original_request(&transaction_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: format!("Failed to get original INVITE request: {}", e)
            })?
            .ok_or_else(|| ApiError::Internal {
                message: "Original INVITE request not found".to_string()
            })?;
        
        // Build 200 OK response with SDP and proper To tag for dialog establishment
        let response = {
            use rvoip_transaction_core::utils::response_builders;
            
            // Use the dialog-aware response builder that adds To tags
            // Get the actual local address from the dialog handle
            let local_addr = self.dialog_handle.dialog_manager.local_address();
            let mut response = response_builders::create_ok_response_with_dialog_info(
                &original_request,
                "server",                    // contact_user
                &local_addr.ip().to_string(), // contact_host - use actual local IP
                Some(local_addr.port())      // contact_port - use actual local port
            );
            
            // Add SDP body if provided
            if let Some(sdp) = &sdp_answer {
                response = response.with_body(sdp.as_bytes().to_vec());
                // Add Content-Type header for SDP
                use rvoip_sip_core::{TypedHeader, types::content_type::ContentType};
                use rvoip_sip_core::parser::headers::content_type::ContentTypeValue;
                response.headers.push(TypedHeader::ContentType(ContentType::new(
                    ContentTypeValue {
                        m_type: "application".to_string(),
                        m_subtype: "sdp".to_string(),
                        parameters: std::collections::HashMap::new(),
                    }
                )));
            }
            
            response
        };
        
        // CRITICAL FIX: Extract the tag from the response BEFORE sending it
        let response_local_tag = response.to()
            .and_then(|to_header| to_header.tag())
            .map(|tag| tag.to_string());
        
        // Send the 200 OK response
        self.dialog_handle.dialog_manager.send_response(&transaction_id, response).await
            .map_err(|e| ApiError::Internal {
                message: format!("Failed to send 200 OK response: {}", e)
            })?;
        
        // Update dialog state to Confirmed and set local tag to match the response
        {
            let mut dialog = self.dialog_handle.dialog_manager.get_dialog_mut(self.call_id())
                .map_err(|e| ApiError::Internal {
                    message: format!("Failed to get dialog for state update: {}", e)
                })?;
            
            // CRITICAL FIX: Use the tag from the response we just sent to ensure consistency
            if dialog.local_tag.is_none() {
                if let Some(response_tag) = response_local_tag {
                    dialog.local_tag = Some(response_tag.clone());
                    info!("Dialog {} local tag set to match response: {}", self.call_id(), response_tag);
                } else {
                    // Fallback: generate tag if response doesn't have one (shouldn't happen)
                    let local_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(local_tag);
                    warn!("Dialog {} fallback tag generation used", self.call_id());
                }
            }
            
            // Transition from Early to Confirmed
            if dialog.state == crate::dialog::DialogState::Early {
                dialog.state = crate::dialog::DialogState::Confirmed;
                info!("Dialog {} transitioned to Confirmed state", self.call_id());
                
                // CRITICAL FIX: Update dialog lookup now that we have both tags
                if let Some(tuple) = dialog.dialog_id_tuple() {
                    use crate::manager::utils::DialogUtils;
                    let key = DialogUtils::create_lookup_key(&tuple.0, &tuple.1, &tuple.2);
                    self.dialog_handle.dialog_manager.dialog_lookup.insert(key, dialog.id.clone());
                    info!("Updated dialog lookup for confirmed dialog {}", dialog.id);
                }
            }
        }
        
        // Emit session coordination event
        if let Some(sdp) = sdp_answer {
            let event = crate::events::SessionCoordinationEvent::CallAnswered {
                dialog_id: self.call_id().clone(),
                session_answer: sdp,
            };
            self.dialog_handle.dialog_manager.emit_session_coordination_event(event).await;
        }
        
        info!("Successfully answered call {}", self.call_id());
        Ok(())
    }
    
    /// Reject the call
    /// 
    /// # Arguments
    /// * `status_code` - SIP status code for rejection
    /// * `reason` - Optional reason phrase
    /// 
    /// # Returns
    /// Success or error
    pub async fn reject(&self, status_code: StatusCode, reason: Option<String>) -> ApiResult<()> {
        info!("Rejecting call {} with status {}", self.call_id(), status_code);
        
        // TODO: This should send an error response when response API is available
        debug!("Call {} would be rejected with status {} reason: {:?}", 
               self.call_id(), status_code, reason);
        
        Ok(())
    }
    
    /// Hang up the call (send BYE)
    pub async fn hangup(&self) -> ApiResult<()> {
        info!("Hanging up call {}", self.call_id());
        self.dialog_handle.terminate().await
    }
    
    /// Put the call on hold
    /// 
    /// # Arguments
    /// * `hold_sdp` - SDP with hold attributes
    /// 
    /// # Returns
    /// Success or error
    pub async fn hold(&self, hold_sdp: Option<String>) -> ApiResult<()> {
        info!("Putting call {} on hold", self.call_id());
        
        // Send re-INVITE with hold SDP
        self.dialog_handle.send_request(Method::Invite, hold_sdp).await?;
        
        Ok(())
    }
    
    /// Resume the call from hold
    /// 
    /// # Arguments
    /// * `resume_sdp` - SDP with active media attributes
    /// 
    /// # Returns
    /// Success or error
    pub async fn resume(&self, resume_sdp: Option<String>) -> ApiResult<()> {
        info!("Resuming call {} from hold", self.call_id());
        
        // Send re-INVITE with active SDP
        self.dialog_handle.send_request(Method::Invite, resume_sdp).await?;
        
        Ok(())
    }
    
    /// Transfer the call
    /// 
    /// # Arguments
    /// * `transfer_target` - URI to transfer the call to
    /// 
    /// # Returns
    /// Success or error
    pub async fn transfer(&self, transfer_target: String) -> ApiResult<()> {
        info!("Transferring call {} to {}", self.call_id(), transfer_target);
        
        // Use the enhanced dialog handle method
        self.dialog_handle.send_refer(transfer_target, None).await?;
        
        Ok(())
    }
    
    /// **NEW**: Advanced transfer with custom REFER body
    /// 
    /// Allows sending custom REFER bodies for advanced transfer scenarios.
    /// 
    /// # Arguments
    /// * `transfer_target` - URI to transfer the call to
    /// * `refer_body` - Custom REFER body with additional headers
    /// 
    /// # Returns
    /// Transaction key for the REFER request
    pub async fn transfer_with_body(&self, transfer_target: String, refer_body: String) -> ApiResult<TransactionKey> {
        info!("Transferring call {} to {} with custom body", self.call_id(), transfer_target);
        
        self.dialog_handle.send_refer(transfer_target, Some(refer_body)).await
    }
    
    /// **NEW**: Send call-related notifications
    /// 
    /// Send NOTIFY requests for call-related events.
    /// 
    /// # Arguments
    /// * `event` - Event type being notified
    /// * `body` - Optional notification body
    /// 
    /// # Returns
    /// Transaction key for the NOTIFY request
    pub async fn notify(&self, event: String, body: Option<String>) -> ApiResult<TransactionKey> {
        info!("Sending call notification for {} event {}", self.call_id(), event);
        
        self.dialog_handle.send_notify(event, body).await
    }
    
    /// **NEW**: Update call media parameters
    /// 
    /// Send UPDATE request to modify media parameters without re-INVITE.
    /// 
    /// # Arguments
    /// * `sdp` - Optional SDP body with new media parameters
    /// 
    /// # Returns
    /// Transaction key for the UPDATE request
    pub async fn update_media(&self, sdp: Option<String>) -> ApiResult<TransactionKey> {
        info!("Updating media for call {}", self.call_id());
        
        self.dialog_handle.send_update(sdp).await
    }
    
    /// **NEW**: Send call information
    /// 
    /// Send INFO request with call-related information.
    /// 
    /// # Arguments
    /// * `info_body` - Information to send
    /// 
    /// # Returns
    /// Transaction key for the INFO request
    pub async fn send_info(&self, info_body: String) -> ApiResult<TransactionKey> {
        info!("Sending call info for {}", self.call_id());
        
        self.dialog_handle.send_info(info_body).await
    }
    
    /// **NEW**: Direct dialog operations for advanced use cases
    
    /// Get dialog state
    pub async fn dialog_state(&self) -> ApiResult<DialogState> {
        self.dialog_handle.state().await
    }
    
    /// Send custom request in dialog
    pub async fn send_request(&self, method: Method, body: Option<String>) -> ApiResult<TransactionKey> {
        self.dialog_handle.send_request_with_key(method, body.map(|s| bytes::Bytes::from(s))).await
    }
    
    /// Send response for transaction
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> ApiResult<()> {
        self.dialog_handle.send_response(transaction_id, response).await
    }
    
    /// Check if the call is still active
    pub async fn is_active(&self) -> bool {
        self.dialog_handle.is_active().await
    }
}

/// Information about a call
///
/// Provides comprehensive metadata about an active call including identifiers,
/// state information, and participant details. This is typically obtained from
/// CallHandle.info() and used for monitoring and debugging purposes.
///
/// ## Examples
///
/// ### Displaying Call Information
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::{CallHandle, CallInfo};
/// use rvoip_dialog_core::dialog::DialogState;
///
/// # async fn example(call: CallHandle) -> Result<(), Box<dyn std::error::Error>> {
/// let info = call.info().await?;
///
/// println!("=== Call Information ===");
/// println!("Call ID: {}", info.call_id);
/// println!("SIP Call-ID: {}", info.call_id_header);
/// println!("From: {} (tag: {:?})", info.local_uri, info.local_tag);
/// println!("To: {} (tag: {:?})", info.remote_uri, info.remote_tag);
/// println!("State: {:?}", info.state);
///
/// match info.state {
///     DialogState::Early => println!("Call is ringing"),
///     DialogState::Confirmed => println!("Call is active"),
///     DialogState::Terminated => println!("Call has ended"),
///     _ => println!("Call in transition"),
/// }
/// # Ok(())
/// # }
/// ```
///
/// ### Conditional Operations Based on Call Info
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::{CallHandle, CallInfo};
/// use rvoip_dialog_core::dialog::DialogState;
///
/// # async fn example(call: CallHandle) -> Result<(), Box<dyn std::error::Error>> {
/// let info = call.info().await?;
///
/// // Only allow transfer if call is confirmed
/// if info.state == DialogState::Confirmed {
///     call.transfer("sip:voicemail@example.com".to_string()).await?;
///     println!("Call transferred to voicemail");
/// } else {
///     println!("Cannot transfer call in state: {:?}", info.state);
/// }
///
/// // Check if this is an outbound call (local tag exists)
/// if info.local_tag.is_some() {
///     println!("This is an outbound call");
/// } else {
///     println!("This is an inbound call");
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct CallInfo {
    /// Call ID (dialog ID)
    pub call_id: DialogId,
    
    /// Current call state
    pub state: DialogState,
    
    /// Local URI
    pub local_uri: String,
    
    /// Remote URI
    pub remote_uri: String,
    
    /// SIP Call-ID header value
    pub call_id_header: String,
    
    /// Local tag
    pub local_tag: Option<String>,
    
    /// Remote tag
    pub remote_tag: Option<String>,
}

/// Dialog events that applications can listen for
///
/// Represents various events that occur during the dialog lifecycle,
/// allowing applications to monitor and react to dialog state changes,
/// incoming requests, and responses.
///
/// ## Event Categories
///
/// - **Lifecycle Events**: Created, StateChanged, Terminated
/// - **Message Events**: RequestReceived, ResponseReceived
///
/// ## Examples
///
/// ### Basic Event Handling
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::DialogEvent;
/// use rvoip_dialog_core::dialog::DialogState;
/// use rvoip_sip_core::Method;
///
/// fn handle_dialog_event(event: DialogEvent) {
///     match event {
///         DialogEvent::Created { dialog_id } => {
///             println!("New dialog created: {}", dialog_id);
///         },
///         DialogEvent::StateChanged { dialog_id, old_state, new_state } => {
///             println!("Dialog {} transitioned from {:?} to {:?}", 
///                     dialog_id, old_state, new_state);
///             
///             if new_state == DialogState::Confirmed {
///                 println!("Dialog is now ready for operations");
///             }
///         },
///         DialogEvent::Terminated { dialog_id, reason } => {
///             println!("Dialog {} ended: {}", dialog_id, reason);
///         },
///         DialogEvent::RequestReceived { dialog_id, method, body } => {
///             println!("Dialog {} received {} request", dialog_id, method);
///             if let Some(body) = body {
///                 println!("Request body: {}", body);
///             }
///         },
///         DialogEvent::ResponseReceived { dialog_id, status_code, body } => {
///             println!("Dialog {} received {} response", dialog_id, status_code);
///             if let Some(body) = body {
///                 println!("Response body: {}", body);
///             }
///         },
///     }
/// }
/// ```
///
/// ### Advanced Event Processing
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::DialogEvent;
/// use rvoip_dialog_core::dialog::DialogState;
/// use rvoip_sip_core::{Method, StatusCode};
/// use std::collections::HashMap;
///
/// struct CallManager {
///     active_calls: HashMap<String, CallInfo>,
/// }
///
/// impl CallManager {
///     fn handle_event(&mut self, event: DialogEvent) {
///         match event {
///             DialogEvent::Created { dialog_id } => {
///                 println!("Tracking new dialog: {}", dialog_id);
///                 // Initialize call tracking
///             },
///             DialogEvent::StateChanged { dialog_id, new_state, .. } => {
///                 match new_state {
///                     DialogState::Confirmed => {
///                         println!("Call {} is now active", dialog_id);
///                         // Start call timer, enable features, etc.
///                     },
///                     DialogState::Terminated => {
///                         println!("Call {} ended", dialog_id);
///                         // Cleanup call resources
///                     },
///                     _ => {}
///                 }
///             },
///             DialogEvent::RequestReceived { dialog_id, method, .. } => {
///                 match method {
///                     Method::Invite => println!("Re-INVITE received on {}", dialog_id),
///                     Method::Bye => println!("BYE received on {}", dialog_id),
///                     Method::Info => println!("INFO received on {}", dialog_id),
///                     _ => println!("Other request: {} on {}", method, dialog_id),
///                 }
///             },
///             DialogEvent::ResponseReceived { dialog_id, status_code, .. } => {
///                 // Pattern match on specific status codes
///                 match status_code {
///                     StatusCode::Ok | StatusCode::Accepted => {
///                         println!("Success response {} on {}", status_code, dialog_id);
///                     },
///                     StatusCode::BadRequest | StatusCode::NotFound | StatusCode::ServerInternalError => {
///                         println!("Error response {} on {}", status_code, dialog_id);
///                     },
///                     _ => {
///                         println!("Other response {} on {}", status_code, dialog_id);
///                     }
///                 }
///             },
///             _ => {}
///         }
///     }
/// }
/// # struct CallInfo;
/// ```
///
/// ### Event Filtering and Routing
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::common::DialogEvent;
/// use rvoip_dialog_core::dialog::{DialogId, DialogState};
///
/// fn route_dialog_event(event: DialogEvent, call_id: &DialogId) {
///     // Only process events for the dialog we care about
///     let event_dialog_id = match &event {
///         DialogEvent::Created { dialog_id } => dialog_id,
///         DialogEvent::StateChanged { dialog_id, .. } => dialog_id,
///         DialogEvent::Terminated { dialog_id, .. } => dialog_id,
///         DialogEvent::RequestReceived { dialog_id, .. } => dialog_id,
///         DialogEvent::ResponseReceived { dialog_id, .. } => dialog_id,
///     };
///
///     if event_dialog_id == call_id {
///         println!("Processing event for our dialog: {:?}", event);
///         
///         // Handle the specific event
///         match event {
///             DialogEvent::StateChanged { new_state: DialogState::Confirmed, .. } => {
///                 println!("Our call is now active!");
///             },
///             DialogEvent::Terminated { reason, .. } => {
///                 println!("Our call ended: {}", reason);
///             },
///             _ => {}
///         }
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub enum DialogEvent {
    /// Dialog was created
    Created {
        dialog_id: DialogId,
    },
    
    /// Dialog state changed
    StateChanged {
        dialog_id: DialogId,
        old_state: DialogState,
        new_state: DialogState,
    },
    
    /// Dialog was terminated
    Terminated {
        dialog_id: DialogId,
        reason: String,
    },
    
    /// Request received in dialog
    RequestReceived {
        dialog_id: DialogId,
        method: Method,
        body: Option<String>,
    },
    
    /// Response received in dialog
    ResponseReceived {
        dialog_id: DialogId,
        status_code: StatusCode,
        body: Option<String>,
    },
} 