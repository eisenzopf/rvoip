//! Dialog-Core API Module
//!
//! This module provides high-level, user-friendly APIs for SIP dialog management.
//! It offers both the legacy split APIs (DialogClient/DialogServer) for backward
//! compatibility and the new unified API (UnifiedDialogApi) that replaces both.
//!
//! ## API Architecture
//!
//! ```text
//! API Layer
//! ├── unified.rs         ← **NEW**: Single API for all scenarios
//! ├── client.rs          ← Legacy: Client-specific operations  
//! ├── server/            ← Legacy: Server-specific operations
//! ├── common.rs          ← Shared: DialogHandle, CallHandle
//! └── config.rs          ← Shared: Configuration types
//! ```
//!
//! ## Recommended Usage
//!
//! **New projects** should use the unified API:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::unified::UnifiedDialogApi;
//! use rvoip_dialog_core::config::DialogManagerConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::hybrid("192.168.1.100:5060".parse()?)
//!     .with_from_uri("sip:pbx@company.com")
//!     .with_domain("company.com")
//!     .with_auto_options()
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let api = UnifiedDialogApi::new(transaction_manager, config).await?;
//! api.start().await?;
//!
//! // Can handle both incoming and outgoing calls
//! # Ok(())
//! # }
//! ```
//!
//! **Existing projects** can continue using the split APIs:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogServer};
//!
//! # async fn legacy_example() -> Result<(), Box<dyn std::error::Error>> {
//! // Client for outgoing calls
//! let client = DialogClient::new("127.0.0.1:0").await?;
//! 
//! // Server for incoming calls  
//! let server = DialogServer::new("0.0.0.0:5060").await?;
//! # Ok(())
//! # }
//! ```

// **NEW**: Unified API (recommended for new projects)
pub mod unified;

// Legacy APIs (for backward compatibility)
pub mod client;
pub mod server;

// Shared functionality
pub mod common;
pub mod config;

// Internal error handling
mod errors;

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::events::SessionCoordinationEvent;
use crate::manager::DialogManager;

// Re-export main API types
pub use client::DialogClient;
pub use server::DialogServer;
pub use unified::UnifiedDialogApi;

// Re-export shared types
pub use common::{DialogHandle, CallHandle, DialogEvent, CallInfo};
pub use config::{ClientConfig, ServerConfig, DialogConfig, Credentials};

// Re-export error types
pub use errors::{ApiError, ApiResult};

/// Shared trait for all dialog APIs
///
/// This trait provides common functionality that all dialog API implementations
/// should support, including lifecycle management, session coordination, and
/// basic statistics.
#[allow(async_fn_in_trait)]
pub trait DialogApi: Send + Sync {
    /// Get access to the underlying dialog manager
    ///
    /// Provides access to the core DialogManager for advanced operations
    /// that aren't available through the high-level API.
    fn dialog_manager(&self) -> &Arc<DialogManager>;
    
    /// Set session coordinator for communication with session-core
    ///
    /// Establishes the communication channel for session management events.
    /// This is essential for proper integration with session-core.
    async fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>) -> ApiResult<()>;
    
    /// Start the dialog API
    ///
    /// Initializes the API for processing SIP messages and events.
    async fn start(&self) -> ApiResult<()>;
    
    /// Stop the dialog API
    ///
    /// Gracefully shuts down the API and terminates all active dialogs.
    async fn stop(&self) -> ApiResult<()>;
    
    /// Get statistics for this API instance
    ///
    /// Returns metrics about dialog counts, call success rates, and performance.
    async fn get_stats(&self) -> DialogStats;
}

/// Statistics for dialog API instances
///
/// Provides metrics about the performance and usage of a dialog API instance.
#[derive(Debug, Clone)]
pub struct DialogStats {
    /// Number of currently active dialogs
    pub active_dialogs: usize,
    
    /// Total number of dialogs created
    pub total_dialogs: u64,
    
    /// Number of successful calls (ended with BYE)
    pub successful_calls: u64,
    
    /// Number of failed calls (ended with error)
    pub failed_calls: u64,
    
    /// Average call duration in seconds
    pub avg_call_duration: f64,
} 