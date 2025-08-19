//! Unified API for DialogManager
//!
//! This module provides a unified, high-level API that replaces the separate
//! DialogClient and DialogServer APIs with a single, comprehensive interface.
//! The behavior is determined by the DialogManagerConfig provided during construction.
//!
//! ## Overview
//!
//! The unified API eliminates the artificial client/server split while maintaining
//! all functionality from both previous APIs. The UnifiedDialogApi provides:
//!
//! - **All Client Operations**: `make_call`, outgoing dialog creation, authentication
//! - **All Server Operations**: `handle_invite`, auto-responses, incoming call handling
//! - **All Shared Operations**: Dialog management, response building, SIP method helpers
//! - **Session Coordination**: Integration with session-core for media management
//! - **Statistics & Monitoring**: Comprehensive metrics and dialog state tracking
//!
//! ## Architecture
//!
//! ```text
//! UnifiedDialogApi
//!        │
//!        ├── Configuration-based behavior
//!        │   ├── Client mode: make_call, create_dialog, auth
//!        │   ├── Server mode: handle_invite, auto-options, domain
//!        │   └── Hybrid mode: all operations available
//!        │
//!        ├── Shared operations (all modes)
//!        │   ├── Dialog management
//!        │   ├── Response building
//!        │   ├── SIP method helpers (BYE, REFER, etc.)
//!        │   └── Session coordination
//!        │
//!        └── Convenience handles
//!            ├── DialogHandle (dialog operations)
//!            └── CallHandle (call-specific operations)
//! ```
//!
//! ## Examples
//!
//! ### Client Mode Usage
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::unified::UnifiedDialogApi;
//! use rvoip_dialog_core::config::DialogManagerConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
//!     .with_from_uri("sip:alice@example.com")
//!     .with_auth("alice", "secret123")
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let api = UnifiedDialogApi::new(transaction_manager, config).await?;
//! api.start().await?;
//!
//! // Make outgoing calls
//! let call = api.make_call(
//!     "sip:alice@example.com",
//!     "sip:bob@example.com",
//!     Some("SDP offer".to_string())
//! ).await?;
//!
//! // Use call operations
//! call.hold(Some("SDP with hold".to_string())).await?;
//! call.transfer("sip:voicemail@example.com".to_string()).await?;
//! call.hangup().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Server Mode Usage
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::unified::UnifiedDialogApi;
//! use rvoip_dialog_core::config::DialogManagerConfig;
//! use rvoip_dialog_core::events::SessionCoordinationEvent;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::server("0.0.0.0:5060".parse()?)
//!     .with_domain("sip.company.com")
//!     .with_auto_options()
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let api = UnifiedDialogApi::new(transaction_manager, config).await?;
//!
//! // Set up session coordination
//! let (session_tx, mut session_rx) = tokio::sync::mpsc::channel(100);
//! api.set_session_coordinator(session_tx).await?;
//! api.start().await?;
//!
//! // Handle incoming calls
//! tokio::spawn(async move {
//!     while let Some(event) = session_rx.recv().await {
//!         match event {
//!             SessionCoordinationEvent::IncomingCall { dialog_id, request, .. } => {
//!                 // Handle the incoming call
//!                 # let source_addr = "127.0.0.1:5060".parse().unwrap();
//!                 if let Ok(call) = api.handle_invite(request, source_addr).await {
//!                     call.answer(Some("SDP answer".to_string())).await.ok();
//!                 }
//!             },
//!             _ => {}
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! ```
//!
//! ### Hybrid Mode Usage
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::unified::UnifiedDialogApi;
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
//! let api = UnifiedDialogApi::new(transaction_manager, config).await?;
//! api.start().await?;
//!
//! // Can both make outgoing calls AND handle incoming calls
//! let outgoing_call = api.make_call(
//!     "sip:pbx@company.com",
//!     "sip:external@provider.com",
//!     None
//! ).await?;
//!
//! // Also handles incoming calls via session coordination
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{info, debug};

use crate::transaction::{TransactionManager, TransactionKey, TransactionEvent};
use rvoip_sip_core::{Request, Response, Method, StatusCode};

use crate::config::DialogManagerConfig;
use crate::manager::unified::UnifiedDialogManager;
use crate::dialog::{DialogId, Dialog, DialogState};
use crate::events::{SessionCoordinationEvent, DialogEvent};
use super::{ApiResult, ApiError, DialogStats, common::{DialogHandle, CallHandle}};

/// Unified Dialog API
///
/// Provides a comprehensive, high-level interface for SIP dialog management
/// that combines all functionality from the previous DialogClient and DialogServer
/// APIs into a single, configuration-driven interface.
///
/// ## Key Features
///
/// - **Mode-based behavior**: Client, Server, or Hybrid operation based on configuration
/// - **Complete SIP support**: All SIP methods and dialog operations
/// - **Session integration**: Built-in coordination with session-core
/// - **Convenience handles**: DialogHandle and CallHandle for easy operation
/// - **Comprehensive monitoring**: Statistics, events, and state tracking
/// - **Thread safety**: Safe to share across async tasks using Arc
///
/// ## Capabilities by Mode
///
/// ### Client Mode
/// - Make outgoing calls (`make_call`)
/// - Create outgoing dialogs (`create_dialog`)
/// - Handle authentication challenges
/// - Send in-dialog requests
/// - Build and send responses (when needed)
///
/// ### Server Mode
/// - Handle incoming calls (`handle_invite`)
/// - Auto-respond to OPTIONS/REGISTER (if configured)
/// - Build and send responses
/// - Send in-dialog requests
/// - Domain-based routing
///
/// ### Hybrid Mode
/// - All client capabilities
/// - All server capabilities
/// - Full bidirectional SIP support
/// - Complete PBX/gateway functionality
#[derive(Debug, Clone)]
pub struct UnifiedDialogApi {
    /// Underlying unified dialog manager
    manager: Arc<UnifiedDialogManager>,
    
    /// Configuration for this API instance
    config: DialogManagerConfig,
}

impl UnifiedDialogApi {
    /// Create a new unified dialog API
    ///
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `config` - Configuration determining the behavior mode
    ///
    /// # Returns
    /// New UnifiedDialogApi instance
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::unified::UnifiedDialogApi;
    /// use rvoip_dialog_core::config::DialogManagerConfig;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
    ///     .with_from_uri("sip:alice@example.com")
    ///     .with_auth("alice", "secret123")
    ///     .build();
    ///
    /// # let transaction_manager = std::sync::Arc::new(unimplemented!());
    /// let api = UnifiedDialogApi::new(transaction_manager, config).await?;
    /// api.start().await?;
    ///
    /// // Make outgoing calls
    /// let call = api.make_call(
    ///     "sip:alice@example.com",
    ///     "sip:bob@example.com",
    ///     Some("SDP offer".to_string())
    /// ).await?;
    ///
    /// // Use call operations
    /// call.hold(Some("SDP with hold".to_string())).await?;
    /// call.transfer("sip:voicemail@example.com".to_string()).await?;
    /// call.hangup().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        config: DialogManagerConfig,
    ) -> ApiResult<Self> {
        info!("Creating UnifiedDialogApi in {:?} mode", Self::mode_name(&config));
        
        let manager = Arc::new(
            UnifiedDialogManager::new(transaction_manager, config.clone()).await
                .map_err(ApiError::from)?
        );
        
        Ok(Self {
            manager,
            config,
        })
    }
    
    /// Create a new unified dialog API with global events (RECOMMENDED)
    ///
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `transaction_events` - Global transaction event receiver
    /// * `config` - Configuration determining the behavior mode
    ///
    /// # Returns
    /// New UnifiedDialogApi instance with proper event consumption
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::unified::UnifiedDialogApi;
    /// use rvoip_dialog_core::config::DialogManagerConfig;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let transaction_manager = std::sync::Arc::new(unimplemented!());
    /// # let transaction_events = tokio::sync::mpsc::channel(100).1;
    /// let config = DialogManagerConfig::server("0.0.0.0:5060".parse()?)
    ///     .with_domain("sip.company.com")
    ///     .build();
    ///
    /// let api = UnifiedDialogApi::with_global_events(
    ///     transaction_manager,
    ///     transaction_events,
    ///     config
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        config: DialogManagerConfig,
    ) -> ApiResult<Self> {
        info!("Creating UnifiedDialogApi with global events in {:?} mode", Self::mode_name(&config));
        
        let manager = Arc::new(
            UnifiedDialogManager::with_global_events(transaction_manager, transaction_events, config.clone()).await
                .map_err(ApiError::from)?
        );
        
        Ok(Self {
            manager,
            config,
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
    
    /// Get the underlying dialog manager
    ///
    /// Provides access to the underlying UnifiedDialogManager for advanced operations.
    pub fn dialog_manager(&self) -> &Arc<UnifiedDialogManager> {
        &self.manager
    }
    
    // ========================================
    // LIFECYCLE MANAGEMENT
    // ========================================
    
    /// Start the dialog API
    ///
    /// Initializes the API for processing SIP messages and events.
    pub async fn start(&self) -> ApiResult<()> {
        info!("Starting UnifiedDialogApi");
        self.manager.start().await.map_err(ApiError::from)
    }
    
    /// Stop the dialog API
    ///
    /// Gracefully shuts down the API and all active dialogs.
    pub async fn stop(&self) -> ApiResult<()> {
        info!("Stopping UnifiedDialogApi");
        self.manager.stop().await.map_err(ApiError::from)
    }
    
    // ========================================
    // SESSION COORDINATION
    // ========================================
    
    /// Set session coordinator
    ///
    /// Establishes communication with session-core for session management.
    /// This is essential for media coordination and call lifecycle management.
    ///
    /// # Arguments
    /// * `sender` - Channel sender for session coordination events
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::unified::UnifiedDialogApi;
    /// use rvoip_dialog_core::events::SessionCoordinationEvent;
    ///
    /// # async fn example(api: UnifiedDialogApi) -> Result<(), Box<dyn std::error::Error>> {
    /// let (session_tx, session_rx) = tokio::sync::mpsc::channel(100);
    /// api.set_session_coordinator(session_tx).await?;
    ///
    /// // Handle session events
    /// tokio::spawn(async move {
    ///     // Process session coordination events
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>) -> ApiResult<()> {
        debug!("Setting session coordinator");
        self.manager.set_session_coordinator(sender).await
    }
    
    /// Set dialog event sender
    ///
    /// Establishes dialog event communication for external consumers.
    pub async fn set_dialog_event_sender(&self, sender: mpsc::Sender<DialogEvent>) -> ApiResult<()> {
        debug!("Setting dialog event sender");
        self.manager.set_dialog_event_sender(sender).await
    }
    
    /// Subscribe to dialog events
    ///
    /// Returns a receiver for monitoring dialog state changes and events.
    pub fn subscribe_to_dialog_events(&self) -> mpsc::Receiver<DialogEvent> {
        self.manager.subscribe_to_dialog_events()
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(api: rvoip_dialog_core::api::unified::UnifiedDialogApi) -> Result<(), Box<dyn std::error::Error>> {
    /// let call = api.make_call(
    ///     "sip:alice@example.com",
    ///     "sip:bob@example.com", 
    ///     Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\n...".to_string())
    /// ).await?;
    /// 
    /// println!("Call created: {}", call.call_id());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn make_call(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
    ) -> ApiResult<CallHandle> {
        self.manager.make_call(from_uri, to_uri, sdp_offer).await
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(api: rvoip_dialog_core::api::unified::UnifiedDialogApi) -> Result<(), Box<dyn std::error::Error>> {
    /// let dialog = api.create_dialog("sip:alice@example.com", "sip:bob@example.com").await?;
    /// 
    /// // Send custom requests within the dialog
    /// dialog.send_info("Custom application data".to_string()).await?;
    /// dialog.send_notify("presence".to_string(), Some("online".to_string())).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_dialog(&self, from_uri: &str, to_uri: &str) -> ApiResult<DialogHandle> {
        self.manager.create_dialog(from_uri, to_uri).await
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(api: rvoip_dialog_core::api::unified::UnifiedDialogApi, request: rvoip_sip_core::Request) -> Result<(), Box<dyn std::error::Error>> {
    /// let source = "192.168.1.100:5060".parse().unwrap();
    /// let call = api.handle_invite(request, source).await?;
    /// 
    /// // Accept the call
    /// call.answer(Some("v=0\r\no=server 789 012 IN IP4 192.168.1.10\r\n...".to_string())).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn handle_invite(&self, request: Request, source: SocketAddr) -> ApiResult<CallHandle> {
        self.manager.handle_invite(request, source).await
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
        self.manager.send_request_in_dialog(dialog_id, method, body).await
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
        self.manager.send_response(transaction_id, response).await
    }
    
    /// Build a response for a transaction
    ///
    /// Constructs a properly formatted SIP response.
    ///
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - HTTP-style status code
    /// * `body` - Optional response body
    ///
    /// # Returns
    /// Constructed response ready to send
    pub async fn build_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        body: Option<String>
    ) -> ApiResult<Response> {
        self.manager.build_response(transaction_id, status_code, body).await
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
        reason: Option<String>
    ) -> ApiResult<()> {
        self.manager.send_status_response(transaction_id, status_code, reason).await
    }
    
    // ========================================
    // SIP METHOD HELPERS (ALL MODES)
    // ========================================
    
    /// Send BYE request to terminate a dialog
    pub async fn send_bye(&self, dialog_id: &DialogId) -> ApiResult<TransactionKey> {
        self.manager.send_bye(dialog_id).await
    }
    
    /// Send REFER request for call transfer
    pub async fn send_refer(
        &self,
        dialog_id: &DialogId,
        target_uri: String,
        refer_body: Option<String>
    ) -> ApiResult<TransactionKey> {
        self.manager.send_refer(dialog_id, target_uri, refer_body).await
    }
    
    /// Send NOTIFY request for event notifications
    pub async fn send_notify(
        &self,
        dialog_id: &DialogId,
        event: String,
        body: Option<String>
    ) -> ApiResult<TransactionKey> {
        self.manager.send_notify(dialog_id, event, body).await
    }
    
    /// Send UPDATE request for media modifications
    pub async fn send_update(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>
    ) -> ApiResult<TransactionKey> {
        self.manager.send_update(dialog_id, sdp).await
    }
    
    /// Send INFO request for application-specific information
    pub async fn send_info(
        &self,
        dialog_id: &DialogId,
        info_body: String
    ) -> ApiResult<TransactionKey> {
        self.manager.send_info(dialog_id, info_body).await
    }
    
    /// Send CANCEL request to cancel a pending INVITE
    ///
    /// This method cancels a pending INVITE transaction that hasn't received a final response.
    /// Only works for dialogs in the Early state (before 200 OK is received).
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog to cancel
    ///
    /// # Returns
    /// Transaction key for the CANCEL request
    ///
    /// # Errors
    /// Returns an error if:
    /// - Dialog is not found
    /// - Dialog is not in Early state
    /// - No pending INVITE transaction found
    pub async fn send_cancel(&self, dialog_id: &DialogId) -> ApiResult<TransactionKey> {
        self.manager.send_cancel(dialog_id).await
    }
    
    // ========================================
    // DIALOG MANAGEMENT (ALL MODES)
    // ========================================
    
    /// Get information about a dialog
    pub async fn get_dialog_info(&self, dialog_id: &DialogId) -> ApiResult<Dialog> {
        self.manager.get_dialog_info(dialog_id).await
    }
    
    /// Get the current state of a dialog
    pub async fn get_dialog_state(&self, dialog_id: &DialogId) -> ApiResult<DialogState> {
        self.manager.get_dialog_state(dialog_id).await
    }
    
    /// Terminate a dialog
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> ApiResult<()> {
        self.manager.terminate_dialog(dialog_id).await
    }
    
    /// List all active dialogs
    pub async fn list_active_dialogs(&self) -> Vec<DialogId> {
        self.manager.list_active_dialogs().await
    }
    
    /// Get a dialog handle for convenient operations
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog ID to create a handle for
    ///
    /// # Returns
    /// DialogHandle for the specified dialog
    pub async fn get_dialog_handle(&self, dialog_id: &DialogId) -> ApiResult<DialogHandle> {
        // Verify dialog exists first
        self.get_dialog_info(dialog_id).await?;
        
        // Create handle using the core dialog manager
        Ok(DialogHandle::new(dialog_id.clone(), Arc::new(self.manager.core().clone())))
    }
    
    /// Get a call handle for convenient call operations
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog ID representing the call
    ///
    /// # Returns
    /// CallHandle for the specified call
    pub async fn get_call_handle(&self, dialog_id: &DialogId) -> ApiResult<CallHandle> {
        // Verify dialog exists first
        self.get_dialog_info(dialog_id).await?;
        
        // Create call handle using the core dialog manager
        Ok(CallHandle::new(dialog_id.clone(), Arc::new(self.manager.core().clone())))
    }
    
    // ========================================
    // MONITORING & STATISTICS
    // ========================================
    
    /// Get comprehensive statistics for this API instance
    ///
    /// Returns detailed statistics including dialog counts, call metrics,
    /// and mode-specific information.
    pub async fn get_stats(&self) -> DialogStats {
        let manager_stats = self.manager.get_stats().await;
        
        DialogStats {
            active_dialogs: manager_stats.active_dialogs,
            total_dialogs: manager_stats.total_dialogs,
            successful_calls: manager_stats.successful_calls,
            failed_calls: manager_stats.failed_calls,
            avg_call_duration: if manager_stats.successful_calls > 0 {
                manager_stats.total_call_duration / manager_stats.successful_calls as f64
            } else {
                0.0
            },
        }
    }
    
    /// Get active dialogs with handles for easy management
    ///
    /// Returns a list of DialogHandle instances for all active dialogs.
    pub async fn active_dialogs(&self) -> Vec<DialogHandle> {
        let dialog_ids = self.list_active_dialogs().await;
        let mut handles = Vec::new();
        
        for dialog_id in dialog_ids {
            if let Ok(handle) = self.get_dialog_handle(&dialog_id).await {
                handles.push(handle);
            }
        }
        
        handles
    }
    
    /// Send ACK for 2xx response to INVITE
    ///
    /// Handles the automatic ACK sending required by RFC 3261 for 200 OK responses to INVITE.
    /// This method ensures proper completion of the 3-way handshake (INVITE → 200 OK → ACK).
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
        self.manager.send_ack_for_2xx_response(dialog_id, original_invite_tx_id, response).await
    }
    
    // ========================================
    // CONVENIENCE METHODS
    // ========================================
    
    /// Check if this API supports outgoing calls
    pub fn supports_outgoing_calls(&self) -> bool {
        self.config.supports_outgoing_calls()
    }
    
    /// Check if this API supports incoming calls
    pub fn supports_incoming_calls(&self) -> bool {
        self.config.supports_incoming_calls()
    }
    
    /// Get the from URI for outgoing requests (if configured)
    pub fn from_uri(&self) -> Option<&str> {
        self.config.from_uri()
    }
    
    /// Get the domain for server operations (if configured)
    pub fn domain(&self) -> Option<&str> {
        self.config.domain()
    }
    
    /// Check if automatic authentication is enabled
    pub fn auto_auth_enabled(&self) -> bool {
        self.config.auto_auth_enabled()
    }
    
    /// Check if automatic OPTIONS response is enabled
    pub fn auto_options_enabled(&self) -> bool {
        self.config.auto_options_enabled()
    }
    
    /// Check if automatic REGISTER response is enabled
    pub fn auto_register_enabled(&self) -> bool {
        self.config.auto_register_enabled()
    }
    
    /// Create a new unified dialog API with automatic transport setup (SIMPLE)
    ///
    /// This is the recommended constructor for most use cases. It automatically
    /// creates and configures the transport and transaction managers internally,
    /// providing a clean high-level API.
    ///
    /// # Arguments
    /// * `config` - Configuration determining the behavior mode and bind address
    ///
    /// # Returns
    /// New UnifiedDialogApi instance with automatic transport setup
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::unified::UnifiedDialogApi;
    /// use rvoip_dialog_core::config::DialogManagerConfig;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
    ///     .with_from_uri("sip:alice@example.com")
    ///     .build();
    ///
    /// let api = UnifiedDialogApi::create(config).await?;
    /// api.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create(config: DialogManagerConfig) -> ApiResult<Self> {
        use crate::transaction::{TransactionManager, transport::{TransportManager, TransportManagerConfig}};
        
        info!("Creating UnifiedDialogApi with automatic transport setup in {:?} mode", Self::mode_name(&config));
        
        // Create transport manager automatically with sensible defaults
        let bind_addr = config.local_address();
        let transport_config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec![bind_addr],
            ..Default::default()
        };
        
        let (mut transport, transport_rx) = TransportManager::new(transport_config).await
            .map_err(|e| ApiError::Internal { 
                message: format!("Failed to create transport manager: {}", e) 
            })?;
            
        transport.initialize().await
            .map_err(|e| ApiError::Internal { 
                message: format!("Failed to initialize transport: {}", e) 
            })?;
        
        // Create transaction manager with global events automatically
        let (transaction_manager, global_rx) = TransactionManager::with_transport_manager(
            transport,
            transport_rx,
            Some(100),
        ).await
            .map_err(|e| ApiError::Internal { 
                message: format!("Failed to create transaction manager: {}", e) 
            })?;
        
        // Create the unified dialog API with all components
        Self::with_global_events(Arc::new(transaction_manager), global_rx, config).await
    }
    
    // ========================================
    // NON-DIALOG OPERATIONS
    // ========================================
    
    /// Send a non-dialog SIP request (for REGISTER, OPTIONS, etc.)
    ///
    /// This method allows sending SIP requests that don't establish or require
    /// a dialog context. Useful for:
    /// - REGISTER requests for endpoint registration
    /// - OPTIONS requests for capability discovery
    /// - MESSAGE requests for instant messaging
    /// - SUBSCRIBE requests for event subscriptions
    ///
    /// # Arguments
    /// * `request` - Complete SIP request to send
    /// * `destination` - Target address to send the request to
    /// * `timeout` - Maximum time to wait for a response
    ///
    /// # Returns
    /// The SIP response received
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::expires::ExpiresExt;
    /// use std::time::Duration;
    ///
    /// # async fn example(api: rvoip_dialog_core::api::unified::UnifiedDialogApi) -> Result<(), Box<dyn std::error::Error>> {
    /// // Build a REGISTER request
    /// let request = SimpleRequestBuilder::register("sip:registrar.example.com")?
    ///     .from("", "sip:alice@example.com", Some("tag123"))
    ///     .to("", "sip:alice@example.com", None)
    ///     .call_id("reg-12345")
    ///     .cseq(1)
    ///     .via("192.168.1.100:5060", "UDP", Some("branch123"))
    ///     .contact("sip:alice@192.168.1.100:5060", None)
    ///     .expires(3600)
    ///     .build();
    ///
    /// let destination = "192.168.1.1:5060".parse()?;
    /// let response = api.send_non_dialog_request(
    ///     request,
    ///     destination,
    ///     Duration::from_secs(32)
    /// ).await?;
    ///
    /// println!("Registration response: {}", response.status_code());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_non_dialog_request(
        &self,
        request: Request,
        destination: SocketAddr,
        timeout: std::time::Duration,
    ) -> ApiResult<Response> {
        debug!("Sending non-dialog {} request to {}", request.method(), destination);
        
        // Create a non-dialog transaction directly with the transaction manager
        let transaction_id = match request.method() {
            Method::Invite => {
                return Err(ApiError::protocol(
                    "INVITE requests must use dialog context. Use make_call() instead."
                ));
            }
            _ => {
                // Create non-INVITE client transaction
                self.manager.core().transaction_manager()
                    .create_non_invite_client_transaction(request, destination)
                    .await
                    .map_err(|e| ApiError::internal(format!("Failed to create transaction: {}", e)))?
            }
        };
        
        // Send the request
        self.manager.core().transaction_manager()
            .send_request(&transaction_id)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to send request: {}", e)))?;
        
        // Wait for final response
        let response = self.manager.core().transaction_manager()
            .wait_for_final_response(&transaction_id, timeout)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to wait for response: {}", e)))?
            .ok_or_else(|| ApiError::network(format!("Request timed out after {:?}", timeout)))?;
        
        debug!("Received response {} for non-dialog request", response.status_code());
        Ok(response)
    }
} 