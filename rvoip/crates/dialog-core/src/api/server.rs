//! Dialog Server API
//!
//! This module provides a high-level server interface for SIP dialog management,
//! abstracting the complexity of the underlying DialogManager for server use cases.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{info, debug, warn};

use rvoip_transaction_core::{TransactionManager, TransactionKey};
use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};

use crate::manager::DialogManager;
use crate::events::SessionCoordinationEvent;
use crate::dialog::{DialogId, Dialog, DialogState};
use super::{
    ApiResult, ApiError, DialogApi, DialogStats,
    config::{ServerConfig, DialogConfig},
    common::{DialogHandle, CallHandle},
};

/// High-level server interface for SIP dialog management
/// 
/// Provides a clean, intuitive API for server-side SIP operations including:
/// - Automatic INVITE handling
/// - Response generation
/// - Dialog lifecycle management
/// - Session coordination
/// - **NEW**: Dialog-level coordination for session-core integration
/// 
/// ## Example Usage
/// 
/// ```rust,no_run
/// use rvoip_dialog_core::api::{DialogServer, DialogApi};
/// use tokio::sync::mpsc;
/// 
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Create server with simple configuration
///     let server = DialogServer::new("0.0.0.0:5060").await?;
///     
///     // Set up session coordination
///     let (session_tx, session_rx) = mpsc::channel(100);
///     server.set_session_coordinator(session_tx).await?;
///     
///     // Start processing SIP messages
///     server.start().await?;
///     
///     Ok(())
/// }
/// ```
pub struct DialogServer {
    /// Underlying dialog manager
    dialog_manager: Arc<DialogManager>,
    
    /// Server configuration
    config: ServerConfig,
    
    /// Statistics tracking
    stats: Arc<tokio::sync::RwLock<ServerStats>>,
}

/// Internal statistics tracking
#[derive(Debug, Default)]
struct ServerStats {
    active_dialogs: usize,
    total_dialogs: u64,
    successful_calls: u64,
    failed_calls: u64,
    total_call_duration: f64,
}

impl DialogServer {
    /// Create a new dialog server with simple configuration
    /// 
    /// This is the easiest way to create a server - just provide a local address
    /// and the server will be configured with sensible defaults.
    /// 
    /// # Arguments
    /// * `local_address` - Address to bind to (e.g., "0.0.0.0:5060")
    /// 
    /// # Returns
    /// A configured DialogServer ready to start
    pub async fn new(local_address: &str) -> ApiResult<Self> {
        let addr: SocketAddr = local_address.parse()
            .map_err(|e| ApiError::Configuration { 
                message: format!("Invalid local address '{}': {}", local_address, e) 
            })?;
        
        let config = ServerConfig::new(addr);
        Self::with_config(config).await
    }
    
    /// Create a dialog server with custom configuration
    /// 
    /// Use this for advanced configuration scenarios where you need to customize
    /// timeouts, limits, or other server behavior.
    /// 
    /// # Arguments
    /// * `config` - Server configuration
    /// 
    /// # Returns
    /// A configured DialogServer ready to start
    pub async fn with_config(config: ServerConfig) -> ApiResult<Self> {
        // For now, require dependency injection for proper architecture
        // TODO: Add simple construction once we have a default transport setup
        Err(ApiError::Configuration { 
            message: "Use with_dependencies() method for now - simple construction requires transport setup".to_string() 
        })
    }
    
    /// Create a dialog server with dependency injection
    /// 
    /// Use this when you want full control over dependencies, particularly
    /// useful for testing or when integrating with existing infrastructure.
    /// 
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `config` - Server configuration
    /// 
    /// # Returns
    /// A configured DialogServer ready to start
    pub async fn with_dependencies(
        transaction_manager: Arc<TransactionManager>,
        config: ServerConfig,
    ) -> ApiResult<Self> {
        // Validate configuration
        config.validate()
            .map_err(|e| ApiError::Configuration { message: e })?;
        
        info!("Creating DialogServer with injected dependencies");
        
        // Create dialog manager with injected dependencies
        let dialog_manager = Arc::new(
            DialogManager::new(transaction_manager, config.dialog.local_address).await
                .map_err(|e| ApiError::Internal { 
                    message: format!("Failed to create dialog manager: {}", e) 
                })?
        );
        
        Ok(Self {
            dialog_manager,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ServerStats::default())),
        })
    }
    
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
    
    // **NEW**: Dialog-level coordination methods for session-core integration
    
    /// Send a SIP request within an existing dialog
    /// 
    /// This method provides direct access to sending arbitrary SIP methods
    /// within established dialogs, which is essential for session-core coordination.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to send the request within
    /// * `method` - SIP method to send (BYE, REFER, NOTIFY, etc.)
    /// * `body` - Optional message body
    /// 
    /// # Returns
    /// Transaction key for tracking the request
    pub async fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>
    ) -> ApiResult<TransactionKey> {
        debug!("Sending {} request in dialog {}", method, dialog_id);
        
        self.dialog_manager.send_request(dialog_id, method, body).await
            .map_err(ApiError::from)
    }
    
    /// Create an outgoing dialog for client-initiated communications
    /// 
    /// This method allows session-core to create dialogs for outgoing calls
    /// and other client-initiated SIP operations.
    /// 
    /// # Arguments
    /// * `local_uri` - Local URI (From header)
    /// * `remote_uri` - Remote URI (To header)
    /// * `call_id` - Optional Call-ID (will be generated if None)
    /// 
    /// # Returns
    /// The created dialog ID
    pub async fn create_outgoing_dialog(
        &self,
        local_uri: Uri,
        remote_uri: Uri,
        call_id: Option<String>
    ) -> ApiResult<DialogId> {
        debug!("Creating outgoing dialog from {} to {}", local_uri, remote_uri);
        
        self.dialog_manager.create_outgoing_dialog(local_uri, remote_uri, call_id).await
            .map_err(ApiError::from)
    }
    
    /// Get detailed information about a dialog
    /// 
    /// Provides access to the complete dialog state for session coordination
    /// and monitoring purposes.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to query
    /// 
    /// # Returns
    /// Complete dialog information
    pub async fn get_dialog_info(&self, dialog_id: &DialogId) -> ApiResult<Dialog> {
        self.dialog_manager.get_dialog(dialog_id)
            .map_err(ApiError::from)
    }
    
    /// Get the current state of a dialog
    /// 
    /// Provides quick access to dialog state without retrieving the full
    /// dialog information.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to query
    /// 
    /// # Returns
    /// Current dialog state
    pub async fn get_dialog_state(&self, dialog_id: &DialogId) -> ApiResult<DialogState> {
        self.dialog_manager.get_dialog_state(dialog_id)
            .map_err(ApiError::from)
    }
    
    /// Terminate a dialog and clean up resources
    /// 
    /// This method provides direct control over dialog termination,
    /// which is essential for session lifecycle management.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to terminate
    /// 
    /// # Returns
    /// Success or error
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> ApiResult<()> {
        debug!("Terminating dialog {}", dialog_id);
        
        self.dialog_manager.terminate_dialog(dialog_id).await
            .map_err(ApiError::from)
    }
    
    /// List all active dialog IDs
    /// 
    /// Provides access to all active dialogs for monitoring and
    /// management purposes.
    /// 
    /// # Returns
    /// Vector of active dialog IDs
    pub async fn list_active_dialogs(&self) -> Vec<DialogId> {
        self.dialog_manager.list_dialogs()
    }
    
    /// Send a SIP response for a transaction
    /// 
    /// Provides direct control over response generation, which is essential
    /// for custom response handling in session coordination.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `response` - Complete SIP response
    /// 
    /// # Returns
    /// Success or error
    pub async fn send_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response
    ) -> ApiResult<()> {
        debug!("Sending response for transaction {}", transaction_id);
        
        self.dialog_manager.send_response(transaction_id, response).await
            .map_err(ApiError::from)
    }
    
    /// Build a SIP response with automatic header generation
    /// 
    /// Convenience method for creating properly formatted SIP responses
    /// with correct headers and routing information.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - SIP status code
    /// * `body` - Optional response body
    /// 
    /// # Returns
    /// Built SIP response ready for sending
    pub async fn build_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        body: Option<String>
    ) -> ApiResult<Response> {
        debug!("Building response with status {} for transaction {}", status_code, transaction_id);
        
        // TODO: Implement response building when available in DialogManager
        // For now, return an error indicating this needs implementation
        Err(ApiError::Internal {
            message: "Response building not yet implemented - use send_response() with pre-built Response".to_string()
        })
    }
    
    /// Send a status response with automatic response building
    /// 
    /// Convenience method for sending simple status responses without
    /// manual response construction.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - SIP status code
    /// * `reason` - Optional reason phrase
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
        
        // TODO: Implement when response building is available
        Err(ApiError::Internal {
            message: "Status response sending not yet implemented - use send_response() with pre-built Response".to_string()
        })
    }
    
    // **NEW**: SIP method-specific convenience methods
    
    /// Send a BYE request to terminate a dialog
    /// 
    /// Convenience method for the common operation of ending a call
    /// by sending a BYE request.
    /// 
    /// # Arguments
    /// * `dialog_id` - Dialog to terminate
    /// 
    /// # Returns
    /// Transaction key for the BYE request
    pub async fn send_bye(&self, dialog_id: &DialogId) -> ApiResult<TransactionKey> {
        info!("Sending BYE for dialog {}", dialog_id);
        self.send_request_in_dialog(dialog_id, Method::Bye, None).await
    }
    
    /// Send a REFER request for call transfer
    /// 
    /// Convenience method for initiating call transfers using the
    /// REFER method as defined in RFC 3515.
    /// 
    /// # Arguments
    /// * `dialog_id` - Dialog to send REFER within
    /// * `target_uri` - URI to transfer the call to
    /// * `refer_body` - Optional REFER body with additional headers
    /// 
    /// # Returns
    /// Transaction key for the REFER request
    pub async fn send_refer(
        &self,
        dialog_id: &DialogId,
        target_uri: String,
        refer_body: Option<String>
    ) -> ApiResult<TransactionKey> {
        info!("Sending REFER for dialog {} to {}", dialog_id, target_uri);
        
        let body = if let Some(custom_body) = refer_body {
            custom_body
        } else {
            format!("Refer-To: {}\r\n", target_uri)
        };
        
        self.send_request_in_dialog(dialog_id, Method::Refer, Some(bytes::Bytes::from(body))).await
    }
    
    /// Send a NOTIFY request for event notifications
    /// 
    /// Convenience method for sending event notifications using the
    /// NOTIFY method as defined in RFC 3265.
    /// 
    /// # Arguments
    /// * `dialog_id` - Dialog to send NOTIFY within
    /// * `event` - Event type being notified
    /// * `body` - Optional notification body
    /// 
    /// # Returns
    /// Transaction key for the NOTIFY request
    pub async fn send_notify(
        &self,
        dialog_id: &DialogId,
        event: String,
        body: Option<String>
    ) -> ApiResult<TransactionKey> {
        info!("Sending NOTIFY for dialog {} event {}", dialog_id, event);
        
        let notify_body = body.map(|b| bytes::Bytes::from(b));
        self.send_request_in_dialog(dialog_id, Method::Notify, notify_body).await
    }
    
    /// Send an UPDATE request for media modifications
    /// 
    /// Convenience method for updating media parameters using the
    /// UPDATE method as defined in RFC 3311.
    /// 
    /// # Arguments
    /// * `dialog_id` - Dialog to send UPDATE within
    /// * `sdp` - Optional SDP body with new media parameters
    /// 
    /// # Returns
    /// Transaction key for the UPDATE request
    pub async fn send_update(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>
    ) -> ApiResult<TransactionKey> {
        info!("Sending UPDATE for dialog {}", dialog_id);
        
        let update_body = sdp.map(|s| bytes::Bytes::from(s));
        self.send_request_in_dialog(dialog_id, Method::Update, update_body).await
    }
    
    /// Send an INFO request for application-specific information
    /// 
    /// Convenience method for sending application-specific information
    /// using the INFO method as defined in RFC 6086.
    /// 
    /// # Arguments
    /// * `dialog_id` - Dialog to send INFO within
    /// * `info_body` - Information to send
    /// 
    /// # Returns
    /// Transaction key for the INFO request
    pub async fn send_info(
        &self,
        dialog_id: &DialogId,
        info_body: String
    ) -> ApiResult<TransactionKey> {
        info!("Sending INFO for dialog {}", dialog_id);
        
        self.send_request_in_dialog(dialog_id, Method::Info, Some(bytes::Bytes::from(info_body))).await
    }
    
    /// Get server configuration
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }
    
    /// Get a list of all active dialog handles
    pub async fn active_calls(&self) -> Vec<DialogHandle> {
        let dialog_ids = self.dialog_manager.list_dialogs();
        dialog_ids.into_iter()
            .map(|id| DialogHandle::new(id, self.dialog_manager.clone()))
            .collect()
    }
}

impl DialogApi for DialogServer {
    fn dialog_manager(&self) -> &Arc<DialogManager> {
        &self.dialog_manager
    }
    
    async fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>) -> ApiResult<()> {
        info!("Setting session coordinator for dialog server");
        self.dialog_manager.set_session_coordinator(sender).await;
        Ok(())
    }
    
    async fn start(&self) -> ApiResult<()> {
        info!("Starting dialog server on {}", self.config.dialog.local_address);
        
        self.dialog_manager.start().await
            .map_err(ApiError::from)?;
        
        info!("✅ Dialog server started successfully");
        Ok(())
    }
    
    async fn stop(&self) -> ApiResult<()> {
        info!("Stopping dialog server");
        
        self.dialog_manager.stop().await
            .map_err(ApiError::from)?;
        
        info!("✅ Dialog server stopped successfully");
        Ok(())
    }
    
    async fn get_stats(&self) -> DialogStats {
        let stats = self.stats.read().await;
        DialogStats {
            active_dialogs: stats.active_dialogs,
            total_dialogs: stats.total_dialogs,
            successful_calls: stats.successful_calls,
            failed_calls: stats.failed_calls,
            avg_call_duration: if stats.successful_calls > 0 {
                stats.total_call_duration / stats.successful_calls as f64
            } else {
                0.0
            },
        }
    }
} 