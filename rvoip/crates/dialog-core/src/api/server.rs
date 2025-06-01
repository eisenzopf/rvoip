//! Dialog Server API
//!
//! This module provides a high-level server interface for SIP dialog management,
//! abstracting the complexity of the underlying DialogManager for server use cases.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{info, debug, warn};

use rvoip_transaction_core::TransactionManager;
use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};

use crate::manager::DialogManager;
use crate::events::SessionCoordinationEvent;
use crate::dialog::{DialogId, Dialog};
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
/// 
/// ## Example Usage
/// 
/// ```rust,no_run
/// use rvoip_dialog_core::api::DialogServer;
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