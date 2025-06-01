//! Common API Types
//!
//! This module provides shared types and handles used across the dialog-core API,
//! offering convenient access to dialog operations and events.

use std::sync::Arc;
use tracing::{debug, info};

use rvoip_sip_core::{Method, StatusCode};
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
        
        // Build REFER body
        let refer_body = format!("Refer-To: {}", transfer_target);
        
        // Send REFER request
        self.dialog_handle.send_request(Method::Refer, Some(refer_body)).await?;
        
        Ok(())
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