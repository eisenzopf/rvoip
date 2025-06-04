//! Dialog Client API
//!
//! This module provides a high-level client interface for SIP dialog management,
//! abstracting the complexity of the underlying DialogManager for client use cases.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{info, debug, warn};

use rvoip_transaction_core::{TransactionManager, TransactionKey, TransactionEvent};
use rvoip_transaction_core::builders::dialog_quick;
use rvoip_sip_core::{Uri, Method, Response, StatusCode};

use crate::manager::DialogManager;
use crate::events::SessionCoordinationEvent;
use crate::dialog::{DialogId, Dialog, DialogState};
use super::{
    ApiResult, ApiError, DialogApi, DialogStats,
    config::ClientConfig,
    common::{DialogHandle, CallHandle},
};

/// High-level client interface for SIP dialog management
/// 
/// Provides a clean, intuitive API for client-side SIP operations including:
/// - Making outgoing calls
/// - Dialog lifecycle management
/// - Request sending
/// - Session coordination
/// - **NEW**: Dialog-level coordination for session-core integration
/// 
/// ## Example Usage
/// 
/// ```rust,no_run
/// use rvoip_dialog_core::api::DialogClient;
/// 
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Create client with simple configuration
///     let client = DialogClient::new("127.0.0.1:0").await?;
///     
///     // Make a call
///     let call = client.make_call(
///         "sip:alice@example.com",
///         "sip:bob@example.com",
///         Some("SDP offer".to_string())
///     ).await?;
///     
///     // Wait for answer or hang up
///     call.hangup().await?;
///     
///     Ok(())
/// }
/// ```
pub struct DialogClient {
    /// Underlying dialog manager
    dialog_manager: Arc<DialogManager>,
    
    /// Client configuration
    config: ClientConfig,
    
    /// Statistics tracking
    stats: Arc<tokio::sync::RwLock<ClientStats>>,
}

/// Internal statistics tracking
#[derive(Debug, Default)]
struct ClientStats {
    active_dialogs: usize,
    total_dialogs: u64,
    successful_calls: u64,
    failed_calls: u64,
    total_call_duration: f64,
}

impl DialogClient {
    /// Create a new dialog client with simple configuration
    /// 
    /// This is the easiest way to create a client - just provide a local address
    /// and the client will be configured with sensible defaults.
    /// 
    /// # Arguments
    /// * `local_address` - Local address to use (e.g., "127.0.0.1:0")
    /// 
    /// # Returns
    /// A configured DialogClient ready to start
    pub async fn new(local_address: &str) -> ApiResult<Self> {
        let addr: SocketAddr = local_address.parse()
            .map_err(|e| ApiError::Configuration { 
                message: format!("Invalid local address '{}': {}", local_address, e) 
            })?;
        
        let config = ClientConfig::new(addr);
        Self::with_config(config).await
    }
    
    /// Create a dialog client with custom configuration
    /// 
    /// **ARCHITECTURAL NOTE**: This method requires dependency injection to maintain
    /// proper separation of concerns. dialog-core should not directly manage transport
    /// concerns - that's the responsibility of transaction-core.
    /// 
    /// Use `with_global_events()` or `with_dependencies()` instead, where you provide
    /// a pre-configured TransactionManager that handles all transport setup.
    /// 
    /// # Arguments
    /// * `config` - Client configuration (for validation and future use)
    /// 
    /// # Returns
    /// An error directing users to the proper dependency injection constructors
    pub async fn with_config(config: ClientConfig) -> ApiResult<Self> {
        // Validate configuration for future use
        config.validate()
            .map_err(|e| ApiError::Configuration { message: e })?;
            
        // Return architectural guidance error
        Err(ApiError::Configuration { 
            message: format!(
                "Simple construction violates architectural separation of concerns. \
                 dialog-core should not manage transport directly. \
                 \nUse dependency injection instead:\
                 \n\n1. with_global_events(transaction_manager, events, config) - RECOMMENDED\
                 \n2. with_dependencies(transaction_manager, config)\
                 \n\nExample setup in your application:\
                 \n  // Set up transport and transaction manager in your app\
                 \n  let (tx_mgr, events) = TransactionManager::with_transport(transport).await?;\
                 \n  let client = DialogClient::with_global_events(tx_mgr, events, config).await?;\
                 \n\nSee examples/ directory for complete setup patterns.",
            )
        })
    }
    
    /// Create a dialog client with dependency injection and global events (RECOMMENDED)
    /// 
    /// This constructor follows the working pattern from transaction-core examples
    /// by using global transaction event subscription for proper event consumption.
    /// 
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `transaction_events` - Global transaction event receiver
    /// * `config` - Client configuration
    /// 
    /// # Returns
    /// A configured DialogClient ready to start
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        config: ClientConfig,
    ) -> ApiResult<Self> {
        // Validate configuration
        config.validate()
            .map_err(|e| ApiError::Configuration { message: e })?;
        
        info!("Creating DialogClient with global transaction events (RECOMMENDED PATTERN)");
        
        // Create dialog manager with global event subscription (ROOT CAUSE FIX)
        let dialog_manager = Arc::new(
            DialogManager::with_global_events(transaction_manager, transaction_events, config.dialog.local_address).await
                .map_err(|e| ApiError::Internal { 
                    message: format!("Failed to create dialog manager with global events: {}", e) 
                })?
        );
        
        Ok(Self {
            dialog_manager,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ClientStats::default())),
        })
    }
    
    /// Create a dialog client with dependency injection
    /// 
    /// Use this when you want full control over dependencies, particularly
    /// useful for testing or when integrating with existing infrastructure.
    /// 
    /// **NOTE**: This method still uses the old individual transaction subscription pattern.
    /// For proper event consumption, use `with_global_events()` instead.
    /// 
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `config` - Client configuration
    /// 
    /// # Returns
    /// A configured DialogClient ready to start
    pub async fn with_dependencies(
        transaction_manager: Arc<TransactionManager>,
        config: ClientConfig,
    ) -> ApiResult<Self> {
        // Validate configuration
        config.validate()
            .map_err(|e| ApiError::Configuration { message: e })?;
        
        info!("Creating DialogClient with injected dependencies");
        warn!("WARNING: Using old DialogManager::new() pattern - consider upgrading to with_global_events() for better reliability");
        
        // Create dialog manager with injected dependencies (OLD PATTERN - may have event issues)
        let dialog_manager = Arc::new(
            DialogManager::new(transaction_manager, config.dialog.local_address).await
                .map_err(|e| ApiError::Internal { 
                    message: format!("Failed to create dialog manager: {}", e) 
                })?
        );
        
        Ok(Self {
            dialog_manager,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ClientStats::default())),
        })
    }
    
    /// Make an outgoing call
    /// 
    /// Creates a new dialog and sends an INVITE request to the target.
    /// 
    /// # Arguments
    /// * `from_uri` - Local URI (who the call is from)
    /// * `to_uri` - Target URI (who to call)
    /// * `sdp_offer` - Optional SDP offer for media negotiation
    /// 
    /// # Returns
    /// A CallHandle for managing the call
    pub async fn make_call(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
    ) -> ApiResult<CallHandle> {
        info!("Making call from {} to {}", from_uri, to_uri);
        
        // Parse URIs
        let local_uri: Uri = from_uri.parse()
            .map_err(|e| ApiError::Configuration { 
                message: format!("Invalid from URI '{}': {}", from_uri, e) 
            })?;
        
        let remote_uri: Uri = to_uri.parse()
            .map_err(|e| ApiError::Configuration { 
                message: format!("Invalid to URI '{}': {}", to_uri, e) 
            })?;
        
        // Create outgoing dialog
        let dialog_id = self.dialog_manager.create_outgoing_dialog(
            local_uri,
            remote_uri,
            None, // Let dialog manager generate call-id
        ).await.map_err(ApiError::from)?;
        
        // Send INVITE request
        let body_bytes = sdp_offer.map(|s| bytes::Bytes::from(s));
        let _transaction_key = self.dialog_manager.send_request(&dialog_id, Method::Invite, body_bytes).await
            .map_err(ApiError::from)?;
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.active_dialogs += 1;
            stats.total_dialogs += 1;
        }
        
        debug!("Call created with dialog ID: {}", dialog_id);
        Ok(CallHandle::new(dialog_id, self.dialog_manager.clone()))
    }
    
    /// Create a new dialog without sending a request
    /// 
    /// Useful for advanced scenarios where you want to create a dialog
    /// and send custom requests.
    /// 
    /// # Arguments
    /// * `from_uri` - Local URI
    /// * `to_uri` - Remote URI
    /// 
    /// # Returns
    /// A DialogHandle for the new dialog
    pub async fn create_dialog(&self, from_uri: &str, to_uri: &str) -> ApiResult<DialogHandle> {
        debug!("Creating dialog from {} to {}", from_uri, to_uri);
        
        // Parse URIs
        let local_uri: Uri = from_uri.parse()
            .map_err(|e| ApiError::Configuration { 
                message: format!("Invalid from URI '{}': {}", from_uri, e) 
            })?;
        
        let remote_uri: Uri = to_uri.parse()
            .map_err(|e| ApiError::Configuration { 
                message: format!("Invalid to URI '{}': {}", to_uri, e) 
            })?;
        
        // Create outgoing dialog
        let dialog_id = self.dialog_manager.create_outgoing_dialog(
            local_uri,
            remote_uri,
            None,
        ).await.map_err(ApiError::from)?;
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.active_dialogs += 1;
            stats.total_dialogs += 1;
        }
        
        Ok(DialogHandle::new(dialog_id, self.dialog_manager.clone()))
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
    /// with correct headers and routing information using Phase 3 dialog functions.
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
        debug!("Building response with status {} for transaction {} using Phase 3 functions", status_code, transaction_id);
        
        // Get original request from transaction manager
        let original_request = self.dialog_manager()
            .transaction_manager()
            .original_request(transaction_id)
            .await
            .map_err(|e| ApiError::Internal { 
                message: format!("Failed to get original request: {}", e) 
            })?
            .ok_or_else(|| ApiError::Internal { 
                message: "No original request found for transaction".to_string() 
            })?;
        
        // Use Phase 3 dialog quick function for instant response creation - ONE LINER!
        let response = dialog_quick::response_for_dialog_transaction(
            transaction_id.to_string(),
            original_request,
            None, // No specific dialog ID
            status_code,
            self.dialog_manager.local_address,
            body,
            None // No custom reason
        ).map_err(|e| ApiError::Internal { 
            message: format!("Failed to build response using Phase 3 functions: {}", e) 
        })?;
        
        debug!("Successfully built response with status {} for transaction {} using Phase 3 functions", status_code, transaction_id);
        Ok(response)
    }
    
    /// Build a dialog-aware response with enhanced context
    /// 
    /// This method provides dialog-aware response building using Phase 3 dialog utilities
    /// to ensure proper response construction for dialog transactions.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `dialog_id` - Dialog ID for context
    /// * `status_code` - SIP status code
    /// * `body` - Optional response body
    /// 
    /// # Returns
    /// Built SIP response with dialog awareness
    pub async fn build_dialog_response(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: &DialogId,
        status_code: StatusCode,
        body: Option<String>
    ) -> ApiResult<Response> {
        debug!("Building dialog-aware response with status {} for transaction {} in dialog {} using Phase 3 functions", 
               status_code, transaction_id, dialog_id);
        
        // Get original request from transaction manager
        let original_request = self.dialog_manager()
            .transaction_manager()
            .original_request(transaction_id)
            .await
            .map_err(|e| ApiError::Internal { 
                message: format!("Failed to get original request: {}", e) 
            })?
            .ok_or_else(|| ApiError::Internal { 
                message: "No original request found for transaction".to_string() 
            })?;
        
        // Use Phase 3 dialog quick function with dialog context - ONE LINER!
        let response = dialog_quick::response_for_dialog_transaction(
            transaction_id.to_string(),
            original_request,
            Some(dialog_id.to_string()),
            status_code,
            self.dialog_manager.local_address,
            body,
            None // No custom reason
        ).map_err(|e| ApiError::Internal { 
            message: format!("Failed to build dialog response using Phase 3 functions: {}", e) 
        })?;
        
        debug!("Successfully built dialog-aware response with status {} for transaction {} in dialog {} using Phase 3 functions", 
               status_code, transaction_id, dialog_id);
        Ok(response)
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
        
        // Build the response using our build_response method
        let response = self.build_response(transaction_id, status_code, reason).await?;
        
        // Send the response using the dialog manager
        self.send_response(transaction_id, response).await?;
        
        debug!("Successfully sent status response {} for transaction {}", status_code, transaction_id);
        Ok(())
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
    
    /// Get client configuration
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }
    
    /// Get a list of all active dialog handles
    pub async fn active_dialogs(&self) -> Vec<DialogHandle> {
        let dialog_ids = self.dialog_manager.list_dialogs();
        dialog_ids.into_iter()
            .map(|id| DialogHandle::new(id, self.dialog_manager.clone()))
            .collect()
    }
}

impl DialogApi for DialogClient {
    fn dialog_manager(&self) -> &Arc<DialogManager> {
        &self.dialog_manager
    }
    
    async fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>) -> ApiResult<()> {
        info!("Setting session coordinator for dialog client");
        self.dialog_manager.set_session_coordinator(sender).await;
        Ok(())
    }
    
    async fn start(&self) -> ApiResult<()> {
        info!("Starting dialog client");
        
        self.dialog_manager.start().await
            .map_err(ApiError::from)?;
        
        info!("✅ Dialog client started successfully");
        Ok(())
    }
    
    async fn stop(&self) -> ApiResult<()> {
        info!("Stopping dialog client");
        
        self.dialog_manager.stop().await
            .map_err(ApiError::from)?;
        
        info!("✅ Dialog client stopped successfully");
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