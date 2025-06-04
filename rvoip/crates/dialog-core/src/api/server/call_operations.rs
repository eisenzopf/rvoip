//! Call Operations for DialogServer
//!
//! This module provides call lifecycle management operations including
//! handling incoming calls, accepting/rejecting calls, and call termination.

use std::net::SocketAddr;
use tracing::{info, debug};

use rvoip_sip_core::{Request, Method, StatusCode};
use crate::dialog::DialogId;
use super::super::{ApiResult, ApiError};
use super::super::common::CallHandle;
use super::core::DialogServer;

/// Call operation implementations for DialogServer
impl DialogServer {
    /// Handle an incoming INVITE request
    /// 
    /// This is typically called automatically when INVITEs are received,
    /// but can also be called manually for testing or custom routing.
    /// 
    /// # Arguments
    /// * `request` - The INVITE request
    /// * `source` - Source address of the request
    /// 
    /// # Returns
    /// A CallHandle for managing the call
    pub async fn handle_invite(&self, request: Request, source: SocketAddr) -> ApiResult<CallHandle> {
        debug!("Handling INVITE from {}", source);
        
        // Delegate to dialog manager
        self.dialog_manager.handle_invite(request.clone(), source).await
            .map_err(ApiError::from)?;
        
        // Create dialog from request
        let dialog_id = self.dialog_manager.create_dialog(&request).await
            .map_err(ApiError::from)?;
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.active_dialogs += 1;
            stats.total_dialogs += 1;
        }
        
        Ok(CallHandle::new(dialog_id, self.dialog_manager.clone()))
    }
    
    /// Accept an incoming call
    /// 
    /// Sends a 200 OK response to an INVITE request.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID for the call
    /// * `sdp_answer` - Optional SDP answer for media negotiation
    /// 
    /// # Returns
    /// Success or error
    pub async fn accept_call(&self, dialog_id: &DialogId, sdp_answer: Option<String>) -> ApiResult<()> {
        info!("Accepting call for dialog {}", dialog_id);
        
        // Build 200 OK response
        // TODO: This should use dialog manager's response building capabilities
        // when they become available in the API
        debug!("Call would be accepted for dialog {} with SDP: {:?}", dialog_id, sdp_answer.is_some());
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.successful_calls += 1;
        }
        
        Ok(())
    }
    
    /// Reject an incoming call
    /// 
    /// Sends an error response to an INVITE request.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID for the call
    /// * `status_code` - SIP status code for rejection
    /// * `reason` - Optional reason phrase
    /// 
    /// # Returns
    /// Success or error
    pub async fn reject_call(
        &self, 
        dialog_id: &DialogId, 
        status_code: StatusCode, 
        reason: Option<String>
    ) -> ApiResult<()> {
        info!("Rejecting call for dialog {} with status {}", dialog_id, status_code);
        
        // TODO: This should use dialog manager's response building capabilities
        debug!("Call would be rejected for dialog {} with status {} reason: {:?}", 
               dialog_id, status_code, reason);
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.failed_calls += 1;
        }
        
        Ok(())
    }
    
    /// Terminate a call
    /// 
    /// Sends a BYE request to end an active call.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID for the call
    /// 
    /// # Returns
    /// Success or error
    pub async fn terminate_call(&self, dialog_id: &DialogId) -> ApiResult<()> {
        info!("Terminating call for dialog {}", dialog_id);
        
        // Send BYE request through dialog manager
        self.dialog_manager.send_request(dialog_id, Method::Bye, None).await
            .map_err(ApiError::from)?;
        
        // Terminate the dialog
        self.dialog_manager.terminate_dialog(dialog_id).await
            .map_err(ApiError::from)?;
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.active_dialogs = stats.active_dialogs.saturating_sub(1);
        }
        
        Ok(())
    }
} 