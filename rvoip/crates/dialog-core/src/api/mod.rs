//! Dialog-Core API Layer
//!
//! This module provides clean, high-level interfaces for SIP dialog management,
//! abstracting the complexity of the underlying DialogManager and providing
//! intuitive developer-friendly APIs.
//!
//! ## Quick Start
//!
//! ### Basic SIP Client
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogApi};
//! use rvoip_transaction_core::TransactionManager;
//! use std::sync::Arc;
//! use tokio::sync::mpsc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Set up transaction manager (transport setup omitted for brevity)
//!     # let transport = unimplemented!(); // Mock transport
//!     let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
//!     
//!     // Create client configuration
//!     let config = rvoip_dialog_core::api::ClientConfig::new("127.0.0.1:0".parse()?)
//!         .with_from_uri("sip:alice@example.com");
//!     
//!     // Create dialog client (simplified for docs)
//!     let client = DialogClient::with_dependencies(tx_mgr, config).await?;
//!     
//!     // Start the client
//!     client.start().await?;
//!     
//!     // Make a call
//!     let call = client.make_call(
//!         "sip:alice@example.com",
//!         "sip:bob@example.com", 
//!         Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n...".to_string())
//!     ).await?;
//!     
//!     // Handle call lifecycle
//!     println!("Call created: {}", call.call_id());
//!     
//!     // Clean up
//!     call.hangup().await?;
//!     client.stop().await?;
//!     
//!     Ok(())
//! }
//! ```
//!
//! ### Basic SIP Server
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi};
//! use rvoip_transaction_core::TransactionManager;
//! use std::sync::Arc;
//! use tokio::sync::mpsc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Set up transaction manager (transport setup omitted for brevity)
//!     # let transport = unimplemented!(); // Mock transport
//!     let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
//!     
//!     // Create server configuration
//!     let config = rvoip_dialog_core::api::ServerConfig::new("0.0.0.0:5060".parse()?)
//!         .with_domain("example.com")
//!         .with_auto_options();
//!     
//!     // Create dialog server (simplified for docs)
//!     let server = DialogServer::with_dependencies(tx_mgr, config).await?;
//!     
//!     // Set up session coordination
//!     let (session_tx, mut session_rx) = mpsc::channel(100);
//!     server.set_session_coordinator(session_tx).await?;
//!     
//!     // Start the server
//!     server.start().await?;
//!     
//!     // Handle incoming sessions
//!     tokio::spawn(async move {
//!         while let Some(event) = session_rx.recv().await {
//!             match event {
//!                 rvoip_dialog_core::events::SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
//!                     println!("Incoming call: {}", dialog_id);
//!                     // Handle call...
//!                 },
//!                 _ => {}
//!             }
//!         }
//!     });
//!     
//!     // Keep server running
//!     tokio::signal::ctrl_c().await?;
//!     server.stop().await?;
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Architecture Overview
//!
//! The Dialog-Core API is organized into several layers:
//!
//! ```text
//! ┌─────────────────────────────────────┐
//! │           Application Layer         │
//! ├─────────────────────────────────────┤
//! │    DialogClient    │ DialogServer   │  ← High-level APIs
//! ├─────────────────────────────────────┤
//! │     DialogHandle   │   CallHandle   │  ← Operation handles
//! ├─────────────────────────────────────┤
//! │         DialogManager               │  ← Core dialog logic
//! ├─────────────────────────────────────┤
//! │       TransactionManager            │  ← Transaction handling
//! ├─────────────────────────────────────┤
//! │        TransportManager             │  ← Network transport
//! └─────────────────────────────────────┘
//! ```
//!
//! ## Design Principles
//!
//! - **Clean Interfaces**: Simple, intuitive method names and signatures
//! - **Error Abstraction**: Simplified error types for common scenarios
//! - **Dependency Injection**: Support for both simple construction and advanced configuration
//! - **Session Integration**: Built-in coordination with session-core
//! - **RFC 3261 Compliance**: All operations follow SIP dialog standards
//! - **Async/Await**: Full async support with proper cancellation
//! - **Memory Safety**: No unsafe code, leverages Rust's ownership system
//!
//! ## Usage Patterns
//!
//! ### Pattern 1: Simple Client Usage
//!
//! For basic use cases where you just need to make/receive calls:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogApi};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Dependency injection setup (see examples/ for details)
//! # let (tx_mgr, config) = setup_dependencies().await?;
//! 
//! let client = DialogClient::with_dependencies(tx_mgr, config).await?;
//! client.start().await?;
//! 
//! // Make a call
//! let call = client.make_call("sip:me@here.com", "sip:you@there.com", None).await?;
//! 
//! // Use the call
//! call.transfer("sip:somewhere@else.com".to_string()).await?;
//! call.hangup().await?;
//! # Ok(())
//! # }
//! # async fn setup_dependencies() -> Result<(std::sync::Arc<rvoip_transaction_core::TransactionManager>, rvoip_dialog_core::api::ClientConfig), Box<dyn std::error::Error>> { unimplemented!() }
//! ```
//!
//! ### Pattern 2: Advanced Dialog Management
//!
//! For applications requiring fine-grained control over dialog state:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogHandle};
//! use rvoip_sip_core::Method;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let (tx_mgr, config) = setup_dependencies().await?;
//! let client = DialogClient::with_dependencies(tx_mgr, config).await?;
//! 
//! // Create dialog without initial request
//! let dialog = client.create_dialog("sip:me@here.com", "sip:you@there.com").await?;
//! 
//! // Send custom requests
//! let tx_key = dialog.send_request(Method::Info, Some("Custom info".to_string())).await?;
//! 
//! // Monitor dialog state
//! let state = dialog.state().await?;
//! println!("Dialog state: {:?}", state);
//! 
//! // Terminate when done
//! dialog.terminate().await?;
//! # Ok(())
//! # }
//! # async fn setup_dependencies() -> Result<(std::sync::Arc<rvoip_transaction_core::TransactionManager>, rvoip_dialog_core::api::ClientConfig), Box<dyn std::error::Error>> { unimplemented!() }
//! ```
//!
//! ### Pattern 3: Server with Session Integration
//!
//! For SIP servers that need to coordinate with media/session layers:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi};
//! use rvoip_dialog_core::events::SessionCoordinationEvent;
//! use tokio::sync::mpsc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let (tx_mgr, config) = setup_dependencies().await?;
//! let server = DialogServer::with_dependencies(tx_mgr, config).await?;
//! 
//! // Set up session coordination channel
//! let (session_tx, mut session_rx) = mpsc::channel(100);
//! server.set_session_coordinator(session_tx).await?;
//! server.start().await?;
//! 
//! // Process session events
//! while let Some(event) = session_rx.recv().await {
//!     match event {
//!         SessionCoordinationEvent::IncomingCall { dialog_id, request, .. } => {
//!             println!("New call: {} from {}", dialog_id, request.from().unwrap().uri());
//!             // Process call...
//!         },
//!         SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
//!             println!("Call {} answered with SDP: {}", dialog_id, session_answer);
//!             // Start media...
//!         },
//!         SessionCoordinationEvent::CallTerminated { dialog_id, reason } => {
//!             println!("Call {} ended: {}", dialog_id, reason);
//!             // Cleanup media...
//!         },
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! # async fn setup_dependencies() -> Result<(std::sync::Arc<rvoip_transaction_core::TransactionManager>, rvoip_dialog_core::api::ServerConfig), Box<dyn std::error::Error>> { unimplemented!() }
//! ```
//!
//! ## Configuration Examples
//!
//! ### Basic Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::{ClientConfig, ServerConfig};
//! use std::time::Duration;
//!
//! // Simple client config
//! let client_config = ClientConfig::new("127.0.0.1:0".parse().unwrap())
//!     .with_from_uri("sip:alice@example.com");
//!
//! // Simple server config  
//! let server_config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
//!     .with_domain("sip.example.com");
//! ```
//!
//! ### Advanced Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::ClientConfig;
//! use std::time::Duration;
//!
//! let config = ClientConfig::new("192.168.1.100:5060".parse().unwrap())
//!     .with_from_uri("sip:user@domain.com")
//!     .with_auth("username", "password");
//!
//! // Customize dialog behavior
//! let mut advanced_config = config;
//! advanced_config.dialog = advanced_config.dialog
//!     .with_timeout(Duration::from_secs(300))
//!     .with_max_dialogs(5000)
//!     .with_user_agent("MyApp/1.0");
//! ```
//!
//! ## Error Handling Patterns
//!
//! The API provides simplified error types for common scenarios:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{ApiError, ApiResult, DialogClient};
//!
//! async fn handle_errors() -> ApiResult<()> {
//!     # // Mock client for example
//!     # use std::sync::Arc;
//!     # let client: DialogClient = unimplemented!();
//!     match client.make_call("invalid", "uri", None).await {
//!         Ok(call) => {
//!             println!("Call successful: {}", call.call_id());
//!         },
//!         Err(ApiError::Configuration { message }) => {
//!             eprintln!("Configuration error: {}", message);
//!         },
//!         Err(ApiError::Network { message }) => {
//!             eprintln!("Network error: {}", message);
//!         },
//!         Err(ApiError::Protocol { message }) => {
//!             eprintln!("SIP protocol error: {}", message);
//!         },
//!         Err(ApiError::Dialog { message }) => {
//!             eprintln!("Dialog error: {}", message);
//!         },
//!         Err(ApiError::Internal { message }) => {
//!             eprintln!("Internal error: {}", message);
//!         },
//!     }
//!     Ok(())
//! }
//! ```
//!
//! ## Best Practices
//!
//! 1. **Always use dependency injection**: Don't use the simple constructors in production
//! 2. **Handle session coordination**: Set up proper event handling for robust applications
//! 3. **Monitor dialog state**: Use the provided statistics and state monitoring
//! 4. **Proper error handling**: Match on specific error types for appropriate responses
//! 5. **Resource cleanup**: Always call stop() and cleanup methods when shutting down
//! 6. **Use handles**: Prefer DialogHandle and CallHandle over direct DialogManager access
//!
//! ## Integration with Other Components
//!
//! - **session-core**: For media session management and SDP negotiation
//! - **transport-core**: For network transport and connection management  
//! - **transaction-core**: For SIP transaction reliability and state machines
//! - **sip-core**: For low-level SIP message parsing and generation

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
///
/// This is the standard Result type used throughout the dialog-core API,
/// providing simplified error handling for application developers.
///
/// ## Examples
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::{ApiResult, ApiError};
///
/// async fn example_function() -> ApiResult<String> {
///     Ok("Success".to_string())
/// }
///
/// # async fn usage() {
/// match example_function().await {
///     Ok(result) => println!("Got: {}", result),
///     Err(ApiError::Configuration { message }) => {
///         eprintln!("Config error: {}", message);
///     },
///     Err(e) => eprintln!("Other error: {}", e),
/// }
/// # }
/// ```
pub type ApiResult<T> = Result<T, ApiError>;

/// Simplified error type for API consumers
///
/// Provides high-level error categories that applications can handle
/// appropriately without needing to understand internal dialog-core details.
///
/// ## Error Categories
///
/// - **Configuration**: Invalid configuration or setup parameters
/// - **Network**: Network connectivity or transport issues  
/// - **Protocol**: SIP protocol violations or parsing errors
/// - **Dialog**: Dialog state or lifecycle errors
/// - **Internal**: Internal implementation errors
///
/// ## Examples
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::{ApiError, ApiResult};
///
/// fn handle_api_error(error: ApiError) {
///     match error {
///         ApiError::Configuration { message } => {
///             eprintln!("Please check your configuration: {}", message);
///         },
///         ApiError::Network { message } => {
///             eprintln!("Network problem (check connectivity): {}", message);
///         },
///         ApiError::Protocol { message } => {
///             eprintln!("SIP protocol issue: {}", message);
///         },
///         ApiError::Dialog { message } => {
///             eprintln!("Dialog state problem: {}", message);
///         },
///         ApiError::Internal { message } => {
///             eprintln!("Internal error (please report): {}", message);
///         },
///     }
/// }
/// ```
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
///
/// This trait provides the foundational operations that both DialogClient
/// and DialogServer support, enabling consistent session management and
/// coordination patterns.
///
/// ## Examples
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::{DialogApi, DialogClient};
/// use tokio::sync::mpsc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let (tx_mgr, config) = setup_dependencies().await?;
/// let client = DialogClient::with_dependencies(tx_mgr, config).await?;
/// 
/// // Use common API functionality
/// client.start().await?;
/// 
/// let stats = client.get_stats().await;
/// println!("Active dialogs: {}", stats.active_dialogs);
/// 
/// client.stop().await?;
/// # Ok(())
/// # }
/// # async fn setup_dependencies() -> Result<(std::sync::Arc<rvoip_transaction_core::TransactionManager>, rvoip_dialog_core::api::ClientConfig), Box<dyn std::error::Error>> { unimplemented!() }
/// ```
pub trait DialogApi {
    /// Get the underlying dialog manager (for advanced use cases)
    ///
    /// Provides access to the underlying DialogManager for advanced scenarios
    /// that require direct access to dialog-core functionality.
    ///
    /// **Note**: Direct DialogManager access should be avoided in favor of the
    /// high-level API methods when possible.
    fn dialog_manager(&self) -> &Arc<DialogManager>;
    
    /// Set session coordinator for integration with session-core
    ///
    /// Establishes the communication channel for session coordination events,
    /// enabling integration with media management layers.
    ///
    /// # Arguments
    /// * `sender` - Channel sender for session coordination events
    ///
    /// # Returns
    /// Success or configuration error
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogApi;
    /// use rvoip_dialog_core::events::SessionCoordinationEvent;
    /// use tokio::sync::mpsc;
    ///
    /// # async fn example(api: impl DialogApi) -> Result<(), Box<dyn std::error::Error>> {
    /// let (session_tx, mut session_rx) = mpsc::channel(100);
    /// api.set_session_coordinator(session_tx).await?;
    ///
    /// // Handle session events
    /// tokio::spawn(async move {
    ///     while let Some(event) = session_rx.recv().await {
    ///         match event {
    ///             SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
    ///                 println!("Incoming call: {}", dialog_id);
    ///             },
    ///             _ => {}
    ///         }
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>) -> impl std::future::Future<Output = ApiResult<()>> + Send;
    
    /// Start the dialog API
    ///
    /// Initializes the dialog API and begins processing SIP messages.
    /// This must be called before the API can handle dialogs.
    ///
    /// # Returns
    /// Success or initialization error
    fn start(&self) -> impl std::future::Future<Output = ApiResult<()>> + Send;
    
    /// Stop the dialog API
    ///
    /// Gracefully shuts down the dialog API and cleans up resources.
    /// All active dialogs will be terminated.
    ///
    /// # Returns  
    /// Success or shutdown error
    fn stop(&self) -> impl std::future::Future<Output = ApiResult<()>> + Send;
    
    /// Get dialog statistics
    ///
    /// Retrieves current statistics about dialog usage and performance,
    /// useful for monitoring and debugging.
    ///
    /// # Returns
    /// Current dialog statistics
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogApi;
    ///
    /// # async fn example(api: impl DialogApi) {
    /// let stats = api.get_stats().await;
    /// println!("Dialogs: {} active, {} total", stats.active_dialogs, stats.total_dialogs);
    /// println!("Calls: {} successful, {} failed", stats.successful_calls, stats.failed_calls);
    /// println!("Average call duration: {:.1}s", stats.avg_call_duration);
    /// # }
    /// ```
    fn get_stats(&self) -> impl std::future::Future<Output = DialogStats> + Send;
}

/// Dialog statistics for monitoring
///
/// Provides metrics about dialog usage and performance for monitoring,
/// debugging, and capacity planning.
///
/// ## Examples
///
/// ```rust
/// use rvoip_dialog_core::api::DialogStats;
///
/// fn display_stats(stats: DialogStats) {
///     println!("=== Dialog Statistics ===");
///     println!("Active dialogs: {}", stats.active_dialogs);
///     println!("Total dialogs: {}", stats.total_dialogs);
///     println!("Success rate: {:.1}%", 
///         100.0 * stats.successful_calls as f64 / (stats.successful_calls + stats.failed_calls) as f64);
///     println!("Average call duration: {:.1}s", stats.avg_call_duration);
/// }
/// ```
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