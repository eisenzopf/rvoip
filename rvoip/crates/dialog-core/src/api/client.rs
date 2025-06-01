//! Dialog Client API
//!
//! This module provides a high-level client interface for SIP dialog management,
//! abstracting the complexity of the underlying DialogManager for client use cases.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{info, debug};

use rvoip_transaction_core::TransactionManager;
use rvoip_sip_core::{Uri, Method};

use crate::manager::DialogManager;
use crate::events::SessionCoordinationEvent;
use crate::dialog::DialogId;
use super::{
    ApiResult, ApiError, DialogApi, DialogStats,
    config::{ClientConfig, DialogConfig},
    common::{DialogHandle, CallHandle},
};

/// High-level client interface for SIP dialog management
/// 
/// Provides a clean, intuitive API for client-side SIP operations including:
/// - Making outgoing calls
/// - Dialog lifecycle management
/// - Request sending
/// - Session coordination
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
    /// Use this for advanced configuration scenarios where you need to customize
    /// authentication, timeouts, or other client behavior.
    /// 
    /// # Arguments
    /// * `config` - Client configuration
    /// 
    /// # Returns
    /// A configured DialogClient ready to start
    pub async fn with_config(config: ClientConfig) -> ApiResult<Self> {
        // For now, require dependency injection for proper architecture
        // TODO: Add simple construction once we have a default transport setup
        Err(ApiError::Configuration { 
            message: "Use with_dependencies() method for now - simple construction requires transport setup".to_string() 
        })
    }
    
    /// Create a dialog client with dependency injection
    /// 
    /// Use this when you want full control over dependencies, particularly
    /// useful for testing or when integrating with existing infrastructure.
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