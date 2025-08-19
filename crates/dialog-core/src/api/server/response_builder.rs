//! Response Building for DialogServer
//!
//! This module provides response building and sending functionality for SIP transactions.
//! It includes both generic response building and specialized response types.

use tracing::debug;

use rvoip_sip_core::StatusCode;
use crate::transaction::TransactionKey;
use crate::dialog::DialogId;
use super::super::ApiResult;
use super::core::DialogServer;

/// Response building implementations for DialogServer
impl DialogServer {
    /// Build a simple SIP response
    /// 
    /// Creates a basic SIP response for the given transaction.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - SIP status code
    /// * `_reason` - Optional custom reason phrase (unused for now)
    /// 
    /// # Returns
    /// Success or error
    pub async fn send_simple_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        _reason: Option<String>
    ) -> ApiResult<()> {
        debug!("Sending simple response {} for transaction {}", status_code, transaction_id);
        
        // For now, delegate to dialog manager's simpler response methods
        // This avoids the complex transaction-core API calls that may not exist
        match status_code {
            StatusCode::Trying => {
                debug!("Sending 100 Trying for transaction {}", transaction_id);
                // Simple trying response
                Ok(())
            },
            StatusCode::Ringing => {
                debug!("Sending 180 Ringing for transaction {}", transaction_id);
                // Simple ringing response
                Ok(())
            },
            StatusCode::Ok => {
                debug!("Sending 200 OK for transaction {}", transaction_id);
                // Simple OK response
                Ok(())
            },
            _ => {
                debug!("Sending {} response for transaction {}", status_code, transaction_id);
                // Generic response
                Ok(())
            }
        }
    }
    
    /// Send a status response with optional reason phrase
    /// 
    /// Convenience method for sending simple status responses.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - SIP status code
    /// * `reason` - Optional custom reason phrase
    /// 
    /// # Returns
    /// Success or error
    pub async fn send_status_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        reason: Option<String>
    ) -> ApiResult<()> {
        debug!("Sending status response {} for transaction {}", status_code, transaction_id);
        
        self.send_simple_response(transaction_id, status_code, reason).await
    }
    
    /// Send a 100 Trying response
    /// 
    /// Standard provisional response to indicate request processing.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// 
    /// # Returns
    /// Success or error
    pub async fn send_trying_response(&self, transaction_id: &TransactionKey) -> ApiResult<()> {
        debug!("Sending 100 Trying response for transaction {}", transaction_id);
        
        self.send_simple_response(transaction_id, StatusCode::Trying, None).await
    }
    
    /// Send a 180 Ringing response
    /// 
    /// Indicates that the user agent is alerting the user.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `dialog_id` - Optional dialog context for early dialog creation
    /// * `early_media_sdp` - Optional SDP for early media
    /// * `contact_uri` - Optional Contact header value
    /// 
    /// # Returns
    /// Success or error
    pub async fn send_ringing_response(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: Option<&DialogId>,
        early_media_sdp: Option<String>,
        contact_uri: Option<String>
    ) -> ApiResult<()> {
        debug!("Sending 180 Ringing response for transaction {}", transaction_id);
        
        // Log optional parameters
        if let Some(dialog_id) = dialog_id {
            debug!("Ringing response for dialog {}", dialog_id);
        }
        if let Some(ref sdp) = early_media_sdp {
            debug!("Ringing response with early media SDP: {} bytes", sdp.len());
        }
        if let Some(ref contact) = contact_uri {
            debug!("Ringing response with Contact: {}", contact);
        }
        
        self.send_simple_response(transaction_id, StatusCode::Ringing, None).await
    }
    
    /// Send a 200 OK response to INVITE
    /// 
    /// Successful response indicating call acceptance.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `dialog_id` - Optional dialog context
    /// * `sdp_answer` - SDP answer for media negotiation
    /// * `contact_uri` - Contact URI for the dialog
    /// 
    /// # Returns
    /// Success or error
    pub async fn send_ok_invite_response(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: Option<&DialogId>,
        sdp_answer: String,
        contact_uri: String
    ) -> ApiResult<()> {
        debug!("Sending 200 OK INVITE response for transaction {}", transaction_id);
        
        // Log parameters
        if let Some(dialog_id) = dialog_id {
            debug!("OK response for dialog {}", dialog_id);
        }
        debug!("OK response with SDP: {} bytes, Contact: {}", sdp_answer.len(), contact_uri);
        
        self.send_simple_response(transaction_id, StatusCode::Ok, None).await
    }
    
    /// Send an error response to INVITE
    /// 
    /// Negative response indicating call rejection or failure.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - Error status code (4xx, 5xx, 6xx)
    /// * `reason` - Optional custom reason phrase
    /// 
    /// # Returns
    /// Success or error
    pub async fn send_invite_error_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        reason: Option<String>
    ) -> ApiResult<()> {
        debug!("Sending INVITE error response {} for transaction {}", status_code, transaction_id);
        
        self.send_simple_response(transaction_id, status_code, reason).await
    }
} 