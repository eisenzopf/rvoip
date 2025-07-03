//! Specialized SIP Methods for DialogServer
//!
//! This module provides specialized SIP method handling for various dialog operations
//! including BYE, REFER, NOTIFY, UPDATE, and INFO methods.

use tracing::{debug, info};

use rvoip_sip_core::Method;
use rvoip_transaction_core::TransactionKey;
use crate::dialog::DialogId;
use super::super::{ApiResult, ApiError};
use super::core::DialogServer;

/// Specialized SIP method implementations for DialogServer
impl DialogServer {
    /// Send a BYE request to terminate a dialog
    /// 
    /// Sends a BYE request within the specified dialog to terminate the call.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog to send BYE within
    /// 
    /// # Returns
    /// Transaction key for tracking the BYE request
    pub async fn send_bye(&self, dialog_id: &DialogId) -> ApiResult<TransactionKey> {
        info!("Sending BYE for dialog {}", dialog_id);
        
        // Send BYE request through dialog manager
        let transaction_key = self.dialog_manager.send_request(dialog_id, Method::Bye, None).await
            .map_err(ApiError::from)?;
        
        // Update statistics - call is ending
        {
            let mut stats = self.stats.write().await;
            stats.active_dialogs = stats.active_dialogs.saturating_sub(1);
        }
        
        Ok(transaction_key)
    }
    
    /// Send a REFER request for call transfer
    /// 
    /// Implements call transfer functionality according to RFC 3515.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog to send REFER within
    /// * `target_uri` - The URI to transfer the call to (Refer-To header)
    /// * `refer_body` - Optional REFER request body
    /// 
    /// # Returns
    /// Transaction key for tracking the REFER request
    pub async fn send_refer(
        &self,
        dialog_id: &DialogId,
        target_uri: String,
        refer_body: Option<String>
    ) -> ApiResult<TransactionKey> {
        debug!("Sending REFER for dialog {} to {}", dialog_id, target_uri);
        
        // Build REFER request body with Refer-To header
        let body = if let Some(custom_body) = refer_body {
            Some(custom_body.into_bytes().into())
        } else {
            // Use target URI as default body or build proper REFER headers
            Some(format!("Refer-To: {}", target_uri).into_bytes().into())
        };
        
        self.dialog_manager.send_request(dialog_id, Method::Refer, body).await
            .map_err(ApiError::from)
    }
    
    /// Send a NOTIFY request for event notification
    /// 
    /// Implements event notification according to RFC 6665.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog to send NOTIFY within
    /// * `event` - Event type (Event header value)
    /// * `body` - Optional notification body
    /// 
    /// # Returns
    /// Transaction key for tracking the NOTIFY request
    pub async fn send_notify(
        &self,
        dialog_id: &DialogId,
        event: String,
        body: Option<String>
    ) -> ApiResult<TransactionKey> {
        debug!("Sending NOTIFY for dialog {} with event {}", dialog_id, event);
        
        let notify_body = body.map(|b| b.into_bytes().into());
        
        self.dialog_manager.send_request(dialog_id, Method::Notify, notify_body).await
            .map_err(ApiError::from)
    }
    
    /// Send an UPDATE request for session modification
    /// 
    /// Implements session modification according to RFC 3311.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog to send UPDATE within
    /// * `sdp` - Optional SDP for session modification
    /// 
    /// # Returns
    /// Transaction key for tracking the UPDATE request
    pub async fn send_update(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>
    ) -> ApiResult<TransactionKey> {
        debug!("Sending UPDATE for dialog {}", dialog_id);
        
        let update_body = sdp.map(|s| s.into_bytes().into());
        
        self.dialog_manager.send_request(dialog_id, Method::Update, update_body).await
            .map_err(ApiError::from)
    }
    
    /// Send an INFO request for application information
    /// 
    /// Implements application-level information exchange according to RFC 6086.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog to send INFO within
    /// * `info_body` - Information payload
    /// 
    /// # Returns
    /// Transaction key for tracking the INFO request
    pub async fn send_info(
        &self,
        dialog_id: &DialogId,
        info_body: String
    ) -> ApiResult<TransactionKey> {
        debug!("Sending INFO for dialog {}", dialog_id);
        
        let body = Some(info_body.into_bytes().into());
        
        self.dialog_manager.send_request(dialog_id, Method::Info, body).await
            .map_err(ApiError::from)
    }
} 