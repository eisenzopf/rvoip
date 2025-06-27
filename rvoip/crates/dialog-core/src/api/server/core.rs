//! Core DialogServer Implementation
//!
//! This module contains the core DialogServer struct and its constructor methods.
//! It handles server initialization, configuration validation, and dependency injection.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{info, warn};

use rvoip_transaction_core::{TransactionManager, TransactionEvent};
use crate::manager::DialogManager;
use crate::events::SessionCoordinationEvent;
use super::super::{ApiResult, ApiError, DialogApi, DialogStats};
use super::super::config::ServerConfig;

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
    pub(crate) dialog_manager: Arc<DialogManager>,
    
    /// Server configuration
    pub(crate) config: ServerConfig,
    
    /// Statistics tracking
    pub(crate) stats: Arc<tokio::sync::RwLock<ServerStats>>,
}

/// Internal statistics tracking
#[derive(Debug, Default)]
pub struct ServerStats {
    pub active_dialogs: usize,
    pub total_dialogs: u64,
    pub successful_calls: u64,
    pub failed_calls: u64,
    pub total_call_duration: f64,
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
    /// **ARCHITECTURAL NOTE**: This method requires dependency injection to maintain
    /// proper separation of concerns. dialog-core should not directly manage transport
    /// concerns - that's the responsibility of transaction-core.
    /// 
    /// Use `with_global_events()` or `with_dependencies()` instead, where you provide
    /// a pre-configured TransactionManager that handles all transport setup.
    /// 
    /// # Arguments
    /// * `config` - Server configuration (for validation and future use)
    /// 
    /// # Returns
    /// An error directing users to the proper dependency injection constructors
    pub async fn with_config(config: ServerConfig) -> ApiResult<Self> {
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
                 \n  let server = DialogServer::with_global_events(tx_mgr, events, config).await?;\
                 \n\nSee examples/ directory for complete setup patterns.",
            )
        })
    }
    
    /// Create a dialog server with dependency injection and global events (RECOMMENDED)
    /// 
    /// This constructor follows the working pattern from transaction-core examples
    /// by using global transaction event subscription for proper event consumption.
    /// 
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `transaction_events` - Global transaction event receiver
    /// * `config` - Server configuration
    /// 
    /// # Returns
    /// A configured DialogServer ready to start
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        config: ServerConfig,
    ) -> ApiResult<Self> {
        // Validate configuration
        config.validate()
            .map_err(|e| ApiError::Configuration { message: e })?;
        
        info!("Creating DialogServer with global transaction events (RECOMMENDED PATTERN)");
        
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
            stats: Arc::new(tokio::sync::RwLock::new(ServerStats::default())),
        })
    }
    
    /// Create a dialog server with dependency injection
    /// 
    /// Use this when you want full control over dependencies, particularly
    /// useful for testing or when integrating with existing infrastructure.
    /// 
    /// **NOTE**: This method still uses the old individual transaction subscription pattern.
    /// For proper event consumption, use `with_global_events()` instead.
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
            stats: Arc::new(tokio::sync::RwLock::new(ServerStats::default())),
        })
    }
    
    /// Get server configuration
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }
}

/// Implementation of DialogApi trait
impl DialogApi for DialogServer {
    fn dialog_manager(&self) -> &Arc<DialogManager> {
        &self.dialog_manager
    }
    
    async fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>) -> ApiResult<()> {
        self.dialog_manager.set_session_coordinator(sender).await;
        Ok(())
    }
    
    async fn start(&self) -> ApiResult<()> {
        info!("Starting DialogServer on {}", self.config.dialog.local_address);
        self.dialog_manager.start().await
            .map_err(ApiError::from)
    }
    
    async fn stop(&self) -> ApiResult<()> {
        info!("Stopping DialogServer");
        self.dialog_manager.stop().await
            .map_err(ApiError::from)
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