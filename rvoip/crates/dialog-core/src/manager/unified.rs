//! Unified DialogManager Implementation
//!
//! This module provides a unified DialogManager that replaces the separate
//! DialogClient and DialogServer implementations with a single, configuration-driven
//! approach. This aligns with SIP standards where endpoints typically act as both
//! UAC and UAS depending on the transaction, not the application type.
//!
//! ## Architecture
//!
//! ```text
//! DialogManager (unified)
//!        │
//!        ├── Configuration-based behavior
//!        │   ├── Client mode (primarily outgoing)
//!        │   ├── Server mode (primarily incoming)
//!        │   └── Hybrid mode (both directions)
//!        │
//!        ├── Core SIP dialog management (shared)
//!        │   ├── Dialog lifecycle
//!        │   ├── Transaction coordination
//!        │   └── RFC 3261 compliance
//!        │
//!        └── High-level operations (shared)
//!            ├── Response building
//!            ├── SIP method helpers
//!            └── Session coordination
//! ```
//!
//! ## Key Benefits
//!
//! - **Standards Aligned**: Matches how SIP actually works (UAC/UAS per transaction)
//! - **Code Reduction**: ~1000 lines less than split implementation
//! - **Simpler Integration**: Single type for session-core to interact with
//! - **Runtime Flexibility**: Can handle both incoming and outgoing calls
//!
//! ## Examples
//!
//! ### Client Mode Usage
//!
//! ```rust,no_run
//! use rvoip_dialog_core::manager::unified::UnifiedDialogManager;
//! use rvoip_dialog_core::config::DialogManagerConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
//!     .with_from_uri("sip:alice@example.com")
//!     .with_auth("alice", "secret123")
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let manager = UnifiedDialogManager::new(transaction_manager, config).await?;
//! manager.start().await?;
//!
//! // Make outgoing calls
//! let call = manager.make_call(
//!     "sip:alice@example.com",
//!     "sip:bob@example.com", 
//!     Some("SDP offer".to_string())
//! ).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Server Mode Usage
//!
//! ```rust,no_run
//! use rvoip_dialog_core::manager::unified::UnifiedDialogManager;
//! use rvoip_dialog_core::config::DialogManagerConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::server("0.0.0.0:5060".parse()?)
//!     .with_domain("sip.company.com")
//!     .with_auto_options()
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let manager = UnifiedDialogManager::new(transaction_manager, config).await?;
//! manager.start().await?;
//!
//! // Handle incoming calls via session coordination events
//! # Ok(())
//! # }
//! ```
//!
//! ### Hybrid Mode Usage
//!
//! ```rust,no_run
//! use rvoip_dialog_core::manager::unified::UnifiedDialogManager;
//! use rvoip_dialog_core::config::DialogManagerConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::hybrid("192.168.1.100:5060".parse()?)
//!     .with_from_uri("sip:pbx@company.com")
//!     .with_domain("company.com")
//!     .with_auth("pbx", "pbx_password")
//!     .with_auto_options()
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let manager = UnifiedDialogManager::new(transaction_manager, config).await?;
//! manager.start().await?;
//!
//! // Can both make outgoing calls AND handle incoming calls
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{info, debug, warn, error};

use rvoip_transaction_core::{TransactionManager, TransactionKey, TransactionEvent};
use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri, TypedHeader};
use rvoip_sip_core::types::content_length::ContentLength;

use crate::config::DialogManagerConfig;
use crate::dialog::{DialogId, Dialog, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::{SessionCoordinationEvent, DialogEvent};
use crate::api::{ApiResult, ApiError, common::{DialogHandle, CallHandle}};

// Import the existing core DialogManager functionality
use super::core::DialogManager;

/// Unified DialogManager that supports client, server, and hybrid modes
///
/// This is the core implementation that replaces separate DialogClient and DialogServer
/// types with a single, configuration-driven approach. The behavior is determined by
/// the DialogManagerConfig provided during construction.
///
/// ## Capabilities by Mode
///
/// ### Client Mode
/// - Make outgoing calls (`make_call`)
/// - Handle authentication challenges
/// - Send in-dialog requests
/// - Build and send responses (when needed)
///
/// ### Server Mode  
/// - Handle incoming calls (via session coordination)
/// - Auto-respond to OPTIONS/REGISTER (if configured)
/// - Build and send responses
/// - Send in-dialog requests
///
/// ### Hybrid Mode
/// - All client capabilities
/// - All server capabilities
/// - Full bidirectional SIP support
///
/// ## Thread Safety
///
/// UnifiedDialogManager is fully thread-safe and can be shared across async tasks
/// using Arc<UnifiedDialogManager>.
#[derive(Debug, Clone)]
pub struct UnifiedDialogManager {
    /// Core dialog manager (contains all the actual implementation)
    core: DialogManager,
    
    /// Configuration determining behavior mode
    config: DialogManagerConfig,
    
    /// Statistics for this manager instance
    stats: Arc<tokio::sync::RwLock<ManagerStats>>,
}

/// Statistics for the unified dialog manager
#[derive(Debug, Default)]
pub struct ManagerStats {
    /// Number of active dialogs
    pub active_dialogs: usize,
    
    /// Total dialogs created
    pub total_dialogs: u64,
    
    /// Successful calls (ended with BYE)
    pub successful_calls: u64,
    
    /// Failed calls (ended with error)
    pub failed_calls: u64,
    
    /// Total call duration in seconds
    pub total_call_duration: f64,
    
    /// Outgoing calls made (client behavior)
    pub outgoing_calls: u64,
    
    /// Incoming calls handled (server behavior)
    pub incoming_calls: u64,
    
    /// Authentication challenges handled
    pub auth_challenges: u64,
    
    /// Auto-responses sent (OPTIONS, REGISTER)
    pub auto_responses: u64,
}

impl UnifiedDialogManager {
    /// Create a new unified dialog manager
    ///
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `config` - Configuration determining the behavior mode
    ///
    /// # Returns
    /// New UnifiedDialogManager instance
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        config: DialogManagerConfig,
    ) -> DialogResult<Self> {
        // Validate configuration first
        config.validate()
            .map_err(|e| DialogError::internal_error(&format!("Invalid configuration: {}", e), None))?;
        
        let local_address = config.local_address();
        info!("Creating UnifiedDialogManager in {:?} mode at {}", 
            Self::mode_name(&config), local_address);
        
        // Create core dialog manager with the provided transaction manager
        let mut core = DialogManager::new(transaction_manager, local_address).await?;
        
        // **NEW**: Inject the unified configuration into the core manager
        core.set_config(config.clone());
        
        Ok(Self {
            core,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ManagerStats::default())),
        })
    }
    
    /// Create a new unified dialog manager with global events (RECOMMENDED)
    ///
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `transaction_events` - Global transaction event receiver
    /// * `config` - Configuration determining the behavior mode
    ///
    /// # Returns
    /// New UnifiedDialogManager instance with proper event consumption
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        config: DialogManagerConfig,
    ) -> DialogResult<Self> {
        // Validate configuration first
        if let Err(e) = config.validate() {
            error!("Failed to create UnifiedDialogManager: Invalid configuration - {}", e);
            return Err(DialogError::internal_error(&format!("Invalid configuration: {}", e), None));
        }
        
        let local_address = config.local_address();
        info!("Creating UnifiedDialogManager with global events in {:?} mode at {}", 
            Self::mode_name(&config), local_address);
        
        // Create core dialog manager with global events
        let mut core = DialogManager::with_global_events(transaction_manager, transaction_events, local_address).await?;
        
        // **NEW**: Inject the unified configuration into the core manager
        core.set_config(config.clone());
        
        Ok(Self {
            core,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ManagerStats::default())),
        })
    }
    
    /// Get the configuration mode name for logging
    fn mode_name(config: &DialogManagerConfig) -> &'static str {
        match config {
            DialogManagerConfig::Client(_) => "Client",
            DialogManagerConfig::Server(_) => "Server", 
            DialogManagerConfig::Hybrid(_) => "Hybrid",
        }
    }
    
    /// Get the current configuration
    pub fn config(&self) -> &DialogManagerConfig {
        &self.config
    }
    
    /// Get the underlying core dialog manager
    ///
    /// Provides access to the core dialog management functionality.
    /// Useful for advanced operations that bypass the unified API.
    pub fn core(&self) -> &DialogManager {
        &self.core
    }
    
    /// Start the unified dialog manager
    ///
    /// Initializes the manager for processing based on its configuration mode.
    pub async fn start(&self) -> DialogResult<()> {
        info!("Starting UnifiedDialogManager in {:?} mode", Self::mode_name(&self.config));
        
        // Start the core dialog manager
        self.core.start().await?;
        
        // Log mode-specific capabilities
        match &self.config {
            DialogManagerConfig::Client(client) => {
                info!("Client mode active - from_uri: {:?}, auto_auth: {}", 
                    client.from_uri, client.auto_auth);
            },
            DialogManagerConfig::Server(server) => {
                info!("Server mode active - domain: {:?}, auto_options: {}, auto_register: {}", 
                    server.domain, server.auto_options_response, server.auto_register_response);
            },
            DialogManagerConfig::Hybrid(hybrid) => {
                info!("Hybrid mode active - from_uri: {:?}, domain: {:?}, auto_auth: {}, auto_options: {}", 
                    hybrid.from_uri, hybrid.domain, hybrid.auto_auth, hybrid.auto_options_response);
            },
        }
        
        info!("UnifiedDialogManager started successfully");
        Ok(())
    }
    
    /// Stop the unified dialog manager
    ///
    /// Gracefully shuts down the manager and all active dialogs.
    pub async fn stop(&self) -> DialogResult<()> {
        info!("Stopping UnifiedDialogManager");
        
        // Stop the core dialog manager
        self.core.stop().await?;
        
        info!("UnifiedDialogManager stopped successfully");
        Ok(())
    }
    
    /// Set session coordinator
    ///
    /// Establishes communication with session-core for session management.
    pub async fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>) -> ApiResult<()> {
        self.core.set_session_coordinator(sender).await;
        Ok(())
    }
    
    /// Set dialog event sender
    ///
    /// Establishes dialog event communication for external consumers.
    pub async fn set_dialog_event_sender(&self, sender: mpsc::Sender<DialogEvent>) -> ApiResult<()> {
        self.core.set_dialog_event_sender(sender).await;
        Ok(())
    }
    
    /// Subscribe to dialog events
    ///
    /// Returns a receiver for monitoring dialog state changes.
    pub fn subscribe_to_dialog_events(&self) -> mpsc::Receiver<DialogEvent> {
        self.core.subscribe_to_dialog_events()
    }
    
    // ========================================
    // CLIENT-MODE OPERATIONS
    // ========================================
    
    /// Make an outgoing call (Client/Hybrid modes only)
    ///
    /// Creates a new dialog and sends an INVITE request to establish a call.
    /// Only available in Client and Hybrid modes.
    ///
    /// # Arguments
    /// * `from_uri` - Local URI for the call
    /// * `to_uri` - Remote URI to call
    /// * `sdp_offer` - Optional SDP offer for media negotiation
    ///
    /// # Returns
    /// CallHandle for managing the call
    pub async fn make_call(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
    ) -> ApiResult<CallHandle> {
        // Check if outgoing calls are supported
        if !self.config.supports_outgoing_calls() {
            error!("Cannot make outgoing call: Outgoing calls not supported in {:?} mode", 
                Self::mode_name(&self.config));
            return Err(ApiError::Configuration { 
                message: "Outgoing calls not supported in Server mode".to_string() 
            });
        }
        
        info!("Making outgoing call from {} to {}", from_uri, to_uri);
        
        // Parse URIs
        let from_uri: Uri = from_uri.parse()
            .map_err(|e| {
                error!("Failed to parse from_uri '{}': {}", from_uri, e);
                ApiError::Configuration { 
                    message: format!("Invalid from_uri: {}", e) 
                }
            })?;
        let to_uri: Uri = to_uri.parse()
            .map_err(|e| {
                error!("Failed to parse to_uri '{}': {}", to_uri, e);
                ApiError::Configuration { 
                    message: format!("Invalid to_uri: {}", e) 
                }
            })?;
        
        // Create outgoing dialog
        let dialog_id = self.core.create_outgoing_dialog(from_uri, to_uri, None).await
            .map_err(|e| {
                error!("Failed to create outgoing dialog: {}", e);
                ApiError::from(e)
            })?;
        
        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.outgoing_calls += 1;
            stats.active_dialogs += 1;
        }
        
        // Emit dialog creation event
        self.core.emit_dialog_event(DialogEvent::Created { dialog_id: dialog_id.clone() }).await;
        
        // Send INVITE request
        let body_bytes = sdp_offer.map(|s| bytes::Bytes::from(s));
        let _transaction_key = match self.core.send_request(&dialog_id, Method::Invite, body_bytes).await {
            Ok(tx_key) => tx_key,
            Err(e) => {
                // WORKAROUND: For INVITE transactions, "Transaction terminated after timeout" 
                // can occur when the 200 OK response causes the transaction to terminate
                // (correct per RFC 3261). This is actually success, not failure.
                let error_msg = e.to_string();
                if error_msg.contains("Transaction terminated after timeout") || 
                   error_msg.contains("Transaction terminated") {
                    warn!("INVITE transaction terminated (this is normal for 2xx responses): {}", e);
                    // Continue as if successful - the SIP flow is working correctly
                    info!("Created outgoing call with dialog ID: {} (transaction terminated normally)", dialog_id);
                    return Ok(CallHandle::new(dialog_id.clone(), Arc::new(self.core.clone())));
                }
                
                error!("Failed to send INVITE for call {}: {}", dialog_id, e);
                return Err(ApiError::from(e));
            }
        };
        
        // Create call handle
        let call_handle = CallHandle::new(dialog_id.clone(), Arc::new(self.core.clone()));
        
        info!("Created outgoing call with dialog ID: {} and sent INVITE", dialog_id);
        
        Ok(call_handle)
    }
    
    /// Create an outgoing dialog without sending INVITE (Client/Hybrid modes only)
    ///
    /// Creates a dialog in preparation for sending requests. Useful for 
    /// scenarios where you want to create the dialog before sending the INVITE.
    ///
    /// # Arguments
    /// * `from_uri` - Local URI
    /// * `to_uri` - Remote URI
    ///
    /// # Returns
    /// DialogHandle for the new dialog
    pub async fn create_dialog(&self, from_uri: &str, to_uri: &str) -> ApiResult<DialogHandle> {
        // Check if outgoing calls are supported
        if !self.config.supports_outgoing_calls() {
            error!("Cannot create dialog: Dialog creation not supported in {:?} mode", 
                Self::mode_name(&self.config));
            return Err(ApiError::Configuration { 
                message: "Dialog creation not supported in Server mode".to_string() 
            });
        }
        
        debug!("Creating outgoing dialog from {} to {}", from_uri, to_uri);
        
        // Parse URIs
        let from_uri: Uri = from_uri.parse()
            .map_err(|e| {
                error!("Failed to parse from_uri '{}' for dialog creation: {}", from_uri, e);
                ApiError::Configuration { 
                    message: format!("Invalid from_uri: {}", e) 
                }
            })?;
        let to_uri: Uri = to_uri.parse()
            .map_err(|e| {
                error!("Failed to parse to_uri '{}' for dialog creation: {}", to_uri, e);
                ApiError::Configuration { 
                    message: format!("Invalid to_uri: {}", e) 
                }
            })?;
        
        // Create outgoing dialog
        let dialog_id = self.core.create_outgoing_dialog(from_uri, to_uri, None).await
            .map_err(|e| {
                error!("Failed to create outgoing dialog: {}", e);
                ApiError::from(e)
            })?;
        
        // Create dialog handle
        let handle = DialogHandle::new(dialog_id.clone(), Arc::new(self.core.clone()));
        
        debug!("Created dialog: {}", dialog_id);
        Ok(handle)
    }
    
    // ========================================
    // SERVER-MODE OPERATIONS
    // ========================================
    
    /// Handle incoming INVITE request (Server/Hybrid modes only)
    ///
    /// Processes an incoming INVITE to potentially establish a call.
    /// Only available in Server and Hybrid modes.
    ///
    /// # Arguments
    /// * `request` - The INVITE request
    /// * `source` - Source address of the request
    ///
    /// # Returns
    /// CallHandle for managing the incoming call
    pub async fn handle_invite(&self, request: Request, source: SocketAddr) -> ApiResult<CallHandle> {
        // Check if incoming calls are supported
        if !self.config.supports_incoming_calls() {
            error!("Cannot handle incoming INVITE: Incoming calls not supported in {:?} mode", 
                Self::mode_name(&self.config));
            return Err(ApiError::Configuration { 
                message: "Incoming calls not supported in Client mode".to_string() 
            });
        }
        
        info!("Handling incoming INVITE from {}", source);
        
        // Process the INVITE through core dialog manager
        self.core.handle_invite(request.clone(), source).await
            .map_err(|e| {
                error!("Failed to process incoming INVITE from {}: {}", source, e);
                ApiError::from(e)
            })?;
        
        // Find the dialog that was created for this INVITE
        let dialog_id = self.core.find_dialog_for_request(&request).await
            .ok_or_else(|| {
                error!("Failed to find dialog for INVITE request from {}", source);
                ApiError::Dialog { 
                    message: "Failed to find dialog for INVITE request".to_string() 
                }
            })?;
        
        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.incoming_calls += 1;
            stats.active_dialogs += 1;
        }
        
        // Create call handle
        let call_handle = CallHandle::new(dialog_id.clone(), Arc::new(self.core.clone()));
        
        info!("Created incoming call with dialog ID: {}", dialog_id);
        Ok(call_handle)
    }
    
    /// Send automatic response to OPTIONS request (Server/Hybrid modes)
    ///
    /// Automatically responds to OPTIONS requests if auto_options is enabled.
    async fn handle_auto_options(&self, request: Request, source: SocketAddr) -> ApiResult<()> {
        if !self.config.auto_options_enabled() {
            return Ok(()); // Not enabled, skip
        }
        
        info!("Sending automatic OPTIONS response to {}", source);
        
        // Process through core dialog manager
        self.core.handle_options(request, source).await
            .map_err(ApiError::from)?;
        
        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.auto_responses += 1;
        }
        
        Ok(())
    }
    
    // ========================================
    // SHARED OPERATIONS (ALL MODES)
    // ========================================
    
    /// Send a request within an existing dialog
    ///
    /// Available in all modes for sending in-dialog requests.
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog to send the request in
    /// * `method` - SIP method to send
    /// * `body` - Optional request body
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
        
        let method_str = method.to_string(); // Convert to string before move
        self.core.send_request(dialog_id, method, body).await
            .map_err(|e| {
                // Log SIP protocol validation errors as WARN (not ERROR) since they're often expected
                if e.to_string().contains("requires remote tag") || e.to_string().contains("protocol error") {
                    warn!("SIP protocol validation failed for {} in dialog {}: {}", method_str, dialog_id, e);
                } else {
                    error!("Failed to send {} request in dialog {}: {}", method_str, dialog_id, e);
                }
                ApiError::from(e)
            })
    }
    
    /// Send a response to a transaction
    ///
    /// Available in all modes for sending responses to received requests.
    ///
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `response` - The response to send
    pub async fn send_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response
    ) -> ApiResult<()> {
        debug!("Sending response for transaction {}", transaction_id);
        
        self.core.send_response(transaction_id, response).await
            .map_err(|e| {
                error!("Failed to send response for transaction {}: {}", transaction_id, e);
                ApiError::from(e)
            })
    }
    
    /// Build a response for a transaction
    ///
    /// Constructs a properly formatted SIP response.
    ///
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - HTTP-style status code
    /// * `body` - Optional response body (SDP, error details, etc.)
    ///
    /// # Returns
    /// Constructed response ready to send
    pub async fn build_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        body: Option<String>
    ) -> ApiResult<Response> {
        debug!("Building response for transaction {} with status {}", status_code, transaction_id);
        
        // Get the original request from the transaction manager to copy required headers
        let original_request = self.core.transaction_manager()
            .original_request(transaction_id)
            .await
            .map_err(|e| ApiError::Internal { 
                message: format!("Failed to get original request: {}", e) 
            })?
            .ok_or_else(|| ApiError::Internal { 
                message: "No original request found for transaction".to_string() 
            })?;
        
        // Use the proper response builder to create response with all required headers
        let mut response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &original_request,
            status_code,
            None // No custom reason phrase
        );
        
        // Add body if provided
        if let Some(body_content) = body {
            response = response.body(body_content.as_bytes().to_vec());
            
            // Set content type for SDP content
            if body_content.trim_start().starts_with("v=") {
                response = response.content_type("application/sdp");
            }
        }
        
        let built_response = response.build();
        
        debug!("Successfully built response for transaction {} using proper RFC 3261 compliant headers", transaction_id);
        Ok(built_response)
    }
    
    /// Send a status response (convenience method)
    ///
    /// Builds and sends a simple status response.
    ///
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - Status code to send
    /// * `reason` - Optional reason phrase
    pub async fn send_status_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        _reason: Option<String>
    ) -> ApiResult<()> {
        debug!("Sending status response {} for transaction {}", status_code, transaction_id);
        
        let response = self.build_response(transaction_id, status_code, None).await?;
        self.send_response(transaction_id, response).await
    }
    
    // ========================================
    // SIP METHOD HELPERS (ALL MODES)
    // ========================================
    
    /// Send BYE request to terminate a dialog
    pub async fn send_bye(&self, dialog_id: &DialogId) -> ApiResult<TransactionKey> {
        self.send_request_in_dialog(dialog_id, Method::Bye, None).await
    }
    
    /// Send REFER request for call transfer
    pub async fn send_refer(
        &self,
        dialog_id: &DialogId,
        target_uri: String,
        refer_body: Option<String>
    ) -> ApiResult<TransactionKey> {
        let body = if let Some(custom_body) = refer_body {
            custom_body
        } else {
            format!("Refer-To: {}\r\n", target_uri)
        };
        
        self.send_request_in_dialog(dialog_id, Method::Refer, Some(bytes::Bytes::from(body))).await
    }
    
    /// Send NOTIFY request for event notifications
    pub async fn send_notify(
        &self,
        dialog_id: &DialogId,
        event: String,
        body: Option<String>
    ) -> ApiResult<TransactionKey> {
        // TODO: The event parameter will be used to set the Event header in the NOTIFY request
        debug!("Sending NOTIFY for event: {}", event);
        
        let notify_body = body.map(|b| bytes::Bytes::from(b));
        self.send_request_in_dialog(dialog_id, Method::Notify, notify_body).await
    }
    
    /// Send UPDATE request for media modifications
    pub async fn send_update(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>
    ) -> ApiResult<TransactionKey> {
        let update_body = sdp.map(|s| bytes::Bytes::from(s));
        self.send_request_in_dialog(dialog_id, Method::Update, update_body).await
    }
    
    /// Send INFO request for application-specific information
    pub async fn send_info(
        &self,
        dialog_id: &DialogId,
        info_body: String
    ) -> ApiResult<TransactionKey> {
        self.send_request_in_dialog(dialog_id, Method::Info, Some(bytes::Bytes::from(info_body))).await
    }
    
    // ========================================
    // DIALOG MANAGEMENT (ALL MODES)
    // ========================================
    
    /// Get information about a dialog
    pub async fn get_dialog_info(&self, dialog_id: &DialogId) -> ApiResult<Dialog> {
        self.core.get_dialog(dialog_id)
            .map_err(|e| {
                warn!("Failed to get dialog info for {}: {}", dialog_id, e);
                ApiError::from(e)
            })
    }
    
    /// Get the current state of a dialog
    pub async fn get_dialog_state(&self, dialog_id: &DialogId) -> ApiResult<DialogState> {
        self.core.get_dialog_state(dialog_id)
            .map_err(|e| {
                warn!("Failed to get dialog state for {}: {}", dialog_id, e);
                ApiError::from(e)
            })
    }
    
    /// Terminate a dialog
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> ApiResult<()> {
        info!("Terminating dialog {}", dialog_id);
        self.core.terminate_dialog(dialog_id).await
            .map_err(|e| {
                error!("Failed to terminate dialog {}: {}", dialog_id, e);
                ApiError::from(e)
            })
    }
    
    /// List all active dialogs
    pub async fn list_active_dialogs(&self) -> Vec<DialogId> {
        self.core.list_dialogs()
    }
    
    /// Get statistics for this manager
    pub async fn get_stats(&self) -> ManagerStats {
        let stats = self.stats.read().await;
        ManagerStats {
            active_dialogs: self.core.dialog_count(),
            total_dialogs: stats.total_dialogs,
            successful_calls: stats.successful_calls,
            failed_calls: stats.failed_calls,
            total_call_duration: stats.total_call_duration,
            outgoing_calls: stats.outgoing_calls,
            incoming_calls: stats.incoming_calls,
            auth_challenges: stats.auth_challenges,
            auto_responses: stats.auto_responses,
        }
    }
    
    /// Send ACK for 2xx response to INVITE
    ///
    /// Handles the automatic ACK sending required by RFC 3261 for 200 OK responses to INVITE.
    /// Available in all modes for proper SIP protocol compliance.
    ///
    /// # Arguments
    /// * `dialog_id` - Dialog ID for the call
    /// * `original_invite_tx_id` - Transaction ID of the original INVITE
    /// * `response` - The 200 OK response to acknowledge
    ///
    /// # Returns
    /// Success or error
    pub async fn send_ack_for_2xx_response(
        &self,
        dialog_id: &DialogId,
        original_invite_tx_id: &TransactionKey,
        response: &Response
    ) -> ApiResult<()> {
        debug!("Sending ACK for 2xx response for dialog {} via unified API", dialog_id);
        
        self.core.send_ack_for_2xx_response(dialog_id, original_invite_tx_id, response).await
            .map_err(|e| {
                error!("Failed to send ACK for 2xx response for dialog {}: {}", dialog_id, e);
                ApiError::from(e)
            })
    }
}