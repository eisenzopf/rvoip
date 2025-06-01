//! Common API Types
//!
//! This module provides shared types and handles used across the dialog-core API,
//! offering convenient access to dialog operations and events.

use std::sync::Arc;
use tracing::{debug, info};

use rvoip_sip_core::{Method, StatusCode, Response};
use rvoip_transaction_core::TransactionKey;
use crate::manager::DialogManager;
use crate::dialog::{DialogId, Dialog, DialogState};
use super::{ApiResult, ApiError};

/// A handle to a SIP dialog for convenient operations
/// 
/// Provides a high-level interface to dialog operations without exposing
/// the underlying DialogManager complexity.
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
        
        // TODO: This should send a 200 OK response when response API is available
        debug!("Call {} would be answered with SDP: {:?}", self.call_id(), sdp_answer.is_some());
        
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