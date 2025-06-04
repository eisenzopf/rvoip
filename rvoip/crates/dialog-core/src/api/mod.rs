//! Dialog-Core API Layer
//!
//! This module provides clean, high-level interfaces for SIP dialog management,
//! abstracting the complexity of the underlying DialogManager and providing
//! intuitive developer-friendly APIs.
//!
//! ## Design Principles
//!
//! - **Clean Interfaces**: Simple, intuitive method names and signatures
//! - **Error Abstraction**: Simplified error types for common scenarios
//! - **Dependency Injection**: Support for both simple construction and advanced configuration
//! - **Session Integration**: Built-in coordination with session-core
//! - **RFC 3261 Compliance**: All operations follow SIP dialog standards

pub mod client;
pub mod server;
pub mod config;
pub mod common;

// Re-export main API types
pub use client::DialogClient;
pub use server::DialogServer;
pub use config::{DialogConfig, ClientConfig, ServerConfig};
pub use common::{DialogHandle, CallHandle, DialogEvent as ApiDialogEvent};

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::errors::DialogError;
use crate::events::SessionCoordinationEvent;
use crate::manager::DialogManager;

/// High-level result type for API operations
pub type ApiResult<T> = Result<T, ApiError>;

/// Simplified error type for API consumers
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Configuration error
    #[error("Configuration error: {message}")]
    Configuration { message: String },
    
    /// Network error
    #[error("Network error: {message}")]
    Network { message: String },
    
    /// Protocol error
    #[error("SIP protocol error: {message}")]
    Protocol { message: String },
    
    /// Dialog error
    #[error("Dialog error: {message}")]
    Dialog { message: String },
    
    /// Internal error
    #[error("Internal error: {message}")]
    Internal { message: String },
}

impl From<DialogError> for ApiError {
    fn from(error: DialogError) -> Self {
        match error {
            DialogError::InternalError { message, .. } => ApiError::Internal { message },
            DialogError::NetworkError { message, .. } => ApiError::Network { message },
            DialogError::ProtocolError { message, .. } => ApiError::Protocol { message },
            DialogError::DialogNotFound { id, .. } => ApiError::Dialog { message: format!("Dialog not found: {}", id) },
            DialogError::TransactionError { message, .. } => ApiError::Internal { message },
            _ => ApiError::Internal { message: error.to_string() },
        }
    }
}

/// Common functionality shared between client and server APIs
pub trait DialogApi {
    /// Get the underlying dialog manager (for advanced use cases)
    fn dialog_manager(&self) -> &Arc<DialogManager>;
    
    /// Set session coordinator for integration with session-core
    fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>) -> impl std::future::Future<Output = ApiResult<()>> + Send;
    
    /// Start the dialog API
    fn start(&self) -> impl std::future::Future<Output = ApiResult<()>> + Send;
    
    /// Stop the dialog API
    fn stop(&self) -> impl std::future::Future<Output = ApiResult<()>> + Send;
    
    /// Get dialog statistics
    fn get_stats(&self) -> impl std::future::Future<Output = DialogStats> + Send;
}

/// Dialog statistics for monitoring
#[derive(Debug, Clone)]
pub struct DialogStats {
    /// Number of active dialogs
    pub active_dialogs: usize,
    
    /// Total dialogs created
    pub total_dialogs: u64,
    
    /// Number of successful calls
    pub successful_calls: u64,
    
    /// Number of failed calls
    pub failed_calls: u64,
    
    /// Average call duration (in seconds)
    pub avg_call_duration: f64,
} 