//! Dialog Client API
//!
//! This module provides a high-level client interface for SIP dialog management,
//! abstracting the complexity of the underlying DialogManager for client use cases.
//!
//! ## Overview
//!
//! The DialogClient is the primary interface for client-side SIP operations, providing
//! a clean, intuitive API that handles the complexities of SIP dialog management while
//! offering powerful features for call control, media negotiation, and session coordination.
//!
//! ### Key Features
//!
//! - **Call Management**: Make outgoing calls, handle responses, manage call lifecycle
//! - **Dialog Operations**: Create, manage, and terminate SIP dialogs
//! - **Session Integration**: Built-in coordination with session-core for media management
//! - **Request/Response Handling**: Send arbitrary SIP methods and handle responses
//! - **Statistics & Monitoring**: Track call metrics and dialog states
//! - **Dependency Injection**: Clean architecture with proper separation of concerns
//!
//! ## Quick Start
//!
//! ### Basic Client Setup
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogApi, ClientConfig};
//! use crate::transaction::TransactionManager;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Set up dependencies (transport setup omitted for brevity)
//!     # let transport = unimplemented!(); // Mock transport
//!     let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
//!     let config = ClientConfig::new("127.0.0.1:0".parse()?)
//!         .with_from_uri("sip:alice@example.com");
//!     
//!     // Create and start the client
//!     let client = DialogClient::with_dependencies(tx_mgr, config).await?;
//!     client.start().await?;
//!     
//!     // Make a call
//!     let call = client.make_call(
//!         "sip:alice@example.com",
//!         "sip:bob@example.com",
//!         Some("v=0\r\no=- 123 456 IN IP4 192.168.1.100\r\n...".to_string())
//!     ).await?;
//!     
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
//! ## Architecture & Design Patterns
//!
//! ### Dependency Injection Pattern (Recommended)
//!
//! The DialogClient follows a clean dependency injection pattern to maintain proper
//! separation of concerns. The client does not manage transport concerns directly:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, ClientConfig};
//! use crate::transaction::TransactionManager;
//! use std::sync::Arc;
//!
//! # async fn setup_example() -> Result<(), Box<dyn std::error::Error>> {
//! // 1. Set up transport layer (your responsibility)
//! # let transport = unimplemented!(); // Your transport implementation
//! 
//! // 2. Create transaction manager with transport
//! let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
//! 
//! // 3. Configure client behavior
//! let config = ClientConfig::new("192.168.1.100:5060".parse()?)
//!     .with_from_uri("sip:user@domain.com")
//!     .with_auth("username", "password");
//! 
//! // 4. Create client with dependencies
//! let client = DialogClient::with_dependencies(tx_mgr, config).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Session Coordination Pattern
//!
//! For applications that need media management, use the session coordination pattern:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogApi};
//! use rvoip_dialog_core::events::SessionCoordinationEvent;
//! use tokio::sync::mpsc;
//!
//! # async fn session_example() -> Result<(), Box<dyn std::error::Error>> {
//! # let (tx_mgr, config) = setup_dependencies().await?;
//! let client = DialogClient::with_dependencies(tx_mgr, config).await?;
//! 
//! // Set up session coordination
//! let (session_tx, mut session_rx) = mpsc::channel(100);
//! client.set_session_coordinator(session_tx).await?;
//! client.start().await?;
//! 
//! // Handle session events
//! tokio::spawn(async move {
//!     while let Some(event) = session_rx.recv().await {
//!         match event {
//!             SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
//!                 println!("Incoming call: {}", dialog_id);
//!                 // Coordinate with media layer
//!             },
//!             SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
//!                 println!("Call answered: {}", dialog_id);
//!                 // Set up media based on SDP answer
//!             },
//!             _ => {}
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! # async fn setup_dependencies() -> Result<(std::sync::Arc<crate::transaction::TransactionManager>, rvoip_dialog_core::api::ClientConfig), Box<dyn std::error::Error>> { unimplemented!() }
//! ```
//!
//! ## Usage Patterns
//!
//! ### Pattern 1: Simple Call Management
//!
//! For basic voice calls with minimal complexity:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::DialogClient;
//!
//! # async fn simple_calls(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
//! // Make a call
//! let call = client.make_call(
//!     "sip:caller@example.com",
//!     "sip:callee@example.com",
//!     Some("SDP offer".to_string())
//! ).await?;
//! 
//! // Basic call operations
//! call.hold(Some("SDP with hold".to_string())).await?;
//! call.resume(Some("SDP with active media".to_string())).await?;
//! call.transfer("sip:transfer@example.com".to_string()).await?;
//! call.hangup().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Pattern 2: Advanced Dialog Management
//!
//! For applications requiring fine-grained control over SIP dialogs:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::DialogClient;
//! use rvoip_sip_core::Method;
//!
//! # async fn advanced_dialogs(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
//! // Create dialog without initial request
//! let dialog = client.create_dialog("sip:me@here.com", "sip:you@there.com").await?;
//! 
//! // Send custom SIP methods
//! client.send_notify(&dialog.id(), "presence".to_string(), Some("online".to_string())).await?;
//! client.send_update(&dialog.id(), Some("Updated SDP".to_string())).await?;
//! client.send_info(&dialog.id(), "Application data".to_string()).await?;
//! 
//! // Monitor dialog state
//! let state = client.get_dialog_state(&dialog.id()).await?;
//! println!("Dialog state: {:?}", state);
//! 
//! // Clean up
//! client.terminate_dialog(&dialog.id()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Pattern 3: Response Handling & Server-like Behavior
//!
//! For applications that need to handle incoming requests and send responses:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::DialogClient;
//! use rvoip_sip_core::StatusCode;
//! use crate::transaction::TransactionKey;
//!
//! # async fn response_handling(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
//! # let transaction_id: TransactionKey = unimplemented!();
//! # let dialog_id = unimplemented!();
//! 
//! // Build and send responses
//! let response = client.build_response(&transaction_id, StatusCode::Ok, Some("Response body".to_string())).await?;
//! client.send_response(&transaction_id, response).await?;
//! 
//! // Or use convenience method
//! client.send_status_response(&transaction_id, StatusCode::Accepted, None).await?;
//! 
//! // Dialog-aware responses
//! let dialog_response = client.build_dialog_response(
//!     &transaction_id, 
//!     &dialog_id, 
//!     StatusCode::Ok, 
//!     Some("Dialog response".to_string())
//! ).await?;
//! client.send_response(&transaction_id, dialog_response).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Error Handling
//!
//! The DialogClient provides comprehensive error handling with specific error types:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, ApiError};
//!
//! # async fn error_handling(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
//! match client.make_call("invalid-uri", "another-invalid", None).await {
//!     Ok(call) => {
//!         println!("Call successful: {}", call.call_id());
//!     },
//!     Err(ApiError::Configuration { message }) => {
//!         eprintln!("Configuration issue: {}", message);
//!         // Fix configuration and retry
//!     },
//!     Err(ApiError::Network { message }) => {
//!         eprintln!("Network problem: {}", message);
//!         // Check connectivity, retry with backoff
//!     },
//!     Err(ApiError::Protocol { message }) => {
//!         eprintln!("SIP protocol error: {}", message);
//!         // Log for debugging, potentially report upstream
//!     },
//!     Err(ApiError::Dialog { message }) => {
//!         eprintln!("Dialog state error: {}", message);
//!         // Handle dialog lifecycle issues
//!     },
//!     Err(ApiError::Internal { message }) => {
//!         eprintln!("Internal error: {}", message);
//!         // Log for debugging, potentially restart client
//!     },
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Configuration Examples
//!
//! ### Production Client Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::ClientConfig;
//! use std::time::Duration;
//!
//! let config = ClientConfig::new("192.168.1.100:5060".parse().unwrap())
//!     .with_from_uri("sip:service@company.com")
//!     .with_auth("service_user", "secure_password");
//!
//! // Customize for production load
//! let mut prod_config = config;
//! prod_config.dialog = prod_config.dialog
//!     .with_timeout(Duration::from_secs(300))
//!     .with_max_dialogs(10000)
//!     .with_user_agent("MyApp/2.0 (Production)");
//! ```
//!
//! ### Development Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::ClientConfig;
//! use std::time::Duration;
//!
//! let dev_config = ClientConfig::new("127.0.0.1:0".parse().unwrap())
//!     .with_from_uri("sip:dev@localhost");
//!
//! // Fast timeouts for testing
//! let mut test_config = dev_config;
//! test_config.dialog = test_config.dialog
//!     .with_timeout(Duration::from_secs(30))
//!     .with_user_agent("MyApp-Dev/1.0");
//! ```
//!
//! ## Integration with Other Components
//!
//! ### Media Layer Integration
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogApi};
//! use rvoip_dialog_core::events::SessionCoordinationEvent;
//!
//! # async fn media_integration(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
//! // Set up session coordination for media management
//! let (session_tx, mut session_rx) = tokio::sync::mpsc::channel(100);
//! client.set_session_coordinator(session_tx).await?;
//!
//! // Handle media-related events
//! tokio::spawn(async move {
//!     while let Some(event) = session_rx.recv().await {
//!         match event {
//!             SessionCoordinationEvent::IncomingCall { dialog_id, request, .. } => {
//!                 // Extract SDP from request, set up media
//!                 println!("Setting up media for call: {}", dialog_id);
//!             },
//!             SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
//!                 // Use SDP answer to configure media streams
//!                 println!("Configuring media with answer: {}", session_answer);
//!             },
//!             _ => {}
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! ```
//!
//! ### Monitoring & Statistics
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogClient, DialogApi};
//!
//! # async fn monitoring(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
//! // Get client statistics
//! let stats = client.get_stats().await;
//! println!("=== Client Statistics ===");
//! println!("Active dialogs: {}", stats.active_dialogs);
//! println!("Total dialogs: {}", stats.total_dialogs);
//! println!("Success rate: {:.1}%", 
//!     100.0 * stats.successful_calls as f64 / (stats.successful_calls + stats.failed_calls) as f64);
//! println!("Average call duration: {:.1}s", stats.avg_call_duration);
//!
//! // List active dialogs
//! let active = client.active_dialogs().await;
//! for dialog in active {
//!     let info = dialog.info().await?;
//!     println!("Dialog {}: {} -> {} ({})", 
//!         dialog.id(), info.local_uri, info.remote_uri, info.state);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Best Practices
//!
//! 1. **Always use dependency injection**: Use `with_dependencies()` or `with_global_events()` in production
//! 2. **Handle session coordination**: Set up proper event handling for media integration
//! 3. **Monitor client health**: Regularly check statistics and active dialog counts
//! 4. **Implement proper error handling**: Match on specific error types for appropriate responses
//! 5. **Clean shutdown**: Always call `stop()` before terminating the application
//! 6. **Use configuration validation**: Call `validate()` on configurations before use
//! 7. **Leverage handles**: Use DialogHandle and CallHandle for dialog-specific operations
//!
//! ## Thread Safety
//!
//! DialogClient is designed to be thread-safe and can be safely shared across async tasks:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::DialogClient;
//! use std::sync::Arc;
//!
//! # async fn thread_safety(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
//! let client = Arc::new(client);
//!
//! // Spawn multiple tasks using the same client
//! let client1 = client.clone();
//! let task1 = tokio::spawn(async move {
//!     client1.make_call("sip:a@test.com", "sip:b@test.com", None).await
//! });
//!
//! let client2 = client.clone();
//! let task2 = tokio::spawn(async move {
//!     client2.make_call("sip:c@test.com", "sip:d@test.com", None).await
//! });
//!
//! // Both tasks can safely use the client concurrently
//! let (call1, call2) = tokio::try_join!(task1, task2)?;
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use tracing::{info, debug, warn};

use crate::transaction::{TransactionManager, TransactionKey, TransactionEvent};
use crate::transaction::builders::dialog_quick;
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
/// The DialogClient provides a comprehensive, easy-to-use interface for client-side
/// SIP operations. It handles the complexities of SIP dialog management while offering
/// powerful features for call control, media negotiation, and session coordination.
///
/// ## Key Capabilities
///
/// - **Call Management**: Create, control, and terminate voice/video calls
/// - **Dialog Operations**: Full SIP dialog lifecycle management
/// - **Media Coordination**: Integration with media layers through session events
/// - **Request/Response Handling**: Send arbitrary SIP methods and build responses
/// - **Authentication**: Built-in support for SIP authentication challenges
/// - **Statistics & Monitoring**: Real-time metrics and dialog state tracking
///
/// ## Constructor Patterns
///
/// The DialogClient supports multiple construction patterns to fit different use cases:
///
/// ### Pattern 1: Dependency Injection (Recommended)
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::{DialogClient, ClientConfig};
/// use crate::transaction::TransactionManager;
/// use std::sync::Arc;
///
/// # async fn dependency_injection() -> Result<(), Box<dyn std::error::Error>> {
/// // Your application manages transport
/// # let transport = unimplemented!(); // Your transport implementation
/// let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
/// let config = ClientConfig::new("127.0.0.1:0".parse()?);
///
/// let client = DialogClient::with_dependencies(tx_mgr, config).await?;
/// # Ok(())
/// # }
/// ```
///
/// ### Pattern 2: Global Events (Best for Event Processing)
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::{DialogClient, ClientConfig};
/// use crate::transaction::{TransactionManager, TransactionEvent};
/// use tokio::sync::mpsc;
/// use std::sync::Arc;
///
/// # async fn global_events() -> Result<(), Box<dyn std::error::Error>> {
/// # let transport = unimplemented!(); // Your transport
/// let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
/// let (events_tx, events_rx) = mpsc::channel(1000);
/// let config = ClientConfig::new("127.0.0.1:0".parse()?);
///
/// let client = DialogClient::with_global_events(tx_mgr, events_rx, config).await?;
/// # Ok(())
/// # }
/// ```
///
/// ## Complete Usage Example
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::{DialogClient, DialogApi, ClientConfig};
/// use crate::transaction::TransactionManager;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Set up dependencies
///     # let transport = unimplemented!(); // Your transport setup
///     let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
///     let config = ClientConfig::new("192.168.1.100:5060".parse()?)
///         .with_from_uri("sip:alice@company.com")
///         .with_auth("alice", "secret123");
///     
///     // Create and start client
///     let client = DialogClient::with_dependencies(tx_mgr, config).await?;
///     client.start().await?;
///     
///     // Make an outgoing call
///     let call = client.make_call(
///         "sip:alice@company.com",
///         "sip:bob@partner.com",
///         Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\n...".to_string())
///     ).await?;
///     
///     println!("Call initiated: {}", call.call_id());
///     
///     // Call operations
///     tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
///     call.hold(Some("SDP with hold attributes".to_string())).await?;
///     
///     tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
///     call.resume(Some("SDP with active media".to_string())).await?;
///     
///     // Transfer the call
///     call.transfer("sip:voicemail@company.com".to_string()).await?;
///     
///     // Get statistics
///     let stats = client.get_stats().await;
///     println!("Client stats: {} active dialogs, {} total", 
///         stats.active_dialogs, stats.total_dialogs);
///     
///     // Clean shutdown
///     client.stop().await?;
///     
///     Ok(())
/// }
/// ```
///
/// ## Advanced Dialog Operations
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::DialogClient;
/// use rvoip_sip_core::{Method, StatusCode};
///
/// # async fn advanced_operations(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
/// // Create a dialog without sending initial request
/// let dialog = client.create_dialog("sip:me@here.com", "sip:you@there.com").await?;
/// 
/// // Send custom SIP methods within the dialog
/// let info_tx = client.send_info(&dialog.id(), "Application-specific data".to_string()).await?;
/// let notify_tx = client.send_notify(&dialog.id(), "presence".to_string(), Some("online".to_string())).await?;
/// let update_tx = client.send_update(&dialog.id(), Some("Updated media SDP".to_string())).await?;
/// 
/// // Monitor dialog state
/// let state = client.get_dialog_state(&dialog.id()).await?;
/// println!("Dialog state: {:?}", state);
/// 
/// // Build and send a response (if acting as UAS)
/// # let transaction_id = unimplemented!();
/// let response = client.build_response(&transaction_id, StatusCode::Ok, Some("Success".to_string())).await?;
/// client.send_response(&transaction_id, response).await?;
/// 
/// // Terminate the dialog
/// client.terminate_dialog(&dialog.id()).await?;
/// # Ok(())
/// # }
/// ```
///
/// ## Session Coordination Integration
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::{DialogClient, DialogApi};
/// use rvoip_dialog_core::events::SessionCoordinationEvent;
/// use tokio::sync::mpsc;
///
/// # async fn session_integration(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
/// // Set up session coordination for media management
/// let (session_tx, mut session_rx) = mpsc::channel(100);
/// client.set_session_coordinator(session_tx).await?;
/// client.start().await?;
///
/// // Handle session events from the client
/// let session_handler = tokio::spawn(async move {
///     while let Some(event) = session_rx.recv().await {
///         match event {
///             SessionCoordinationEvent::IncomingCall { dialog_id, request, .. } => {
///                 println!("Incoming call on dialog {}", dialog_id);
///                 // Extract SDP, set up media, send response
///             },
///             SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
///                 println!("Call {} answered", dialog_id);
///                 // Configure media streams based on SDP answer
///             },
///             SessionCoordinationEvent::CallTerminated { dialog_id, reason } => {
///                 println!("Call {} terminated: {}", dialog_id, reason);
///                 // Clean up media resources
///             },
///             _ => {}
///         }
///     }
/// });
///
/// // Make calls while session handler processes events
/// let call = client.make_call("sip:me@here.com", "sip:you@there.com", None).await?;
/// 
/// // Session events will be automatically generated and handled
/// tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
/// call.hangup().await?;
/// 
/// # Ok(())
/// # }
/// ```
///
/// ## Error Handling Strategies
///
/// ```rust,no_run
/// use rvoip_dialog_core::api::{DialogClient, ApiError};
/// use tokio::time::{sleep, Duration};
///
/// # async fn error_handling_strategies(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
/// // Retry logic for network errors
/// let mut attempts = 0;
/// let max_attempts = 3;
///
/// loop {
///     match client.make_call("sip:me@here.com", "sip:you@there.com", None).await {
///         Ok(call) => {
///             println!("Call successful: {}", call.call_id());
///             break;
///         },
///         Err(ApiError::Network { message }) if attempts < max_attempts => {
///             attempts += 1;
///             println!("Network error (attempt {}): {}", attempts, message);
///             sleep(Duration::from_secs(2u64.pow(attempts))).await; // Exponential backoff
///             continue;
///         },
///         Err(ApiError::Configuration { message }) => {
///             eprintln!("Configuration error: {}", message);
///             // Fix configuration and restart
///             break;
///         },
///         Err(ApiError::Protocol { message }) => {
///             eprintln!("SIP protocol error: {}", message);
///             // Log for debugging, continue with other operations
///             break;
///         },
///         Err(e) => {
///             eprintln!("Unrecoverable error: {}", e);
///             break;
///         }
///     }
/// }
/// # Ok(())
/// # }
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
    /// This is the **recommended** constructor for production applications. It uses global 
    /// transaction event subscription which provides better event handling reliability and 
    /// prevents event loss that can occur with per-transaction subscriptions.
    ///
    /// ## Why Global Events?
    ///
    /// - **Reliable Event Processing**: All transaction events flow through a single channel
    /// - **No Event Loss**: Events are queued and processed in order
    /// - **Better Resource Management**: Single event loop handles all transactions
    /// - **Cleaner Architecture**: Separation between event generation and consumption
    ///
    /// ## Usage Pattern
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::{DialogClient, DialogApi, ClientConfig};
    /// use crate::transaction::{TransactionManager, TransactionEvent};
    /// use tokio::sync::mpsc;
    /// use std::sync::Arc;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     // Set up transport layer (your responsibility)
    ///     # let transport = unimplemented!(); // Your transport implementation
    ///     
    ///     // Create transaction manager
    ///     let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
    ///     
    ///     // Set up global event channel
    ///     let (event_tx, event_rx) = mpsc::channel(1000);
    ///     
    ///     // Configure client
    ///     let config = ClientConfig::new("192.168.1.100:5060".parse()?)
    ///         .with_from_uri("sip:client@company.com")
    ///         .with_auth("client_user", "password123");
    ///     
    ///     // Create client with global events
    ///     let client = DialogClient::with_global_events(tx_mgr, event_rx, config).await?;
    ///     
    ///     // Start client and begin processing
    ///     client.start().await?;
    ///     
    ///     // Client is now ready for operations
    ///     let call = client.make_call(
    ///         "sip:client@company.com",
    ///         "sip:target@partner.com",
    ///         None
    ///     ).await?;
    ///     
    ///     // Clean shutdown
    ///     call.hangup().await?;
    ///     client.stop().await?;
    ///     
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ## Advanced Event Processing
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::{DialogClient, ClientConfig, DialogApi};
    /// use crate::transaction::{TransactionManager, TransactionEvent};
    /// use tokio::sync::mpsc;
    /// use std::sync::Arc;
    ///
    /// # async fn advanced_event_processing() -> Result<(), Box<dyn std::error::Error>> {
    /// # let transport = unimplemented!();
    /// let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
    /// let (event_tx, event_rx) = mpsc::channel(1000);
    /// let config = ClientConfig::new("127.0.0.1:0".parse()?);
    ///
    /// // Create client
    /// let client = DialogClient::with_global_events(tx_mgr.clone(), event_rx, config).await?;
    ///
    /// // You can also handle transaction events directly if needed
    /// tokio::spawn(async move {
    ///     // This would be your custom transaction event processor
    ///     // The client gets its own copy of events through the channel
    /// });
    ///
    /// client.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    /// 
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager with transport
    /// * `transaction_events` - Global transaction event receiver channel
    /// * `config` - Client configuration with network and authentication settings
    /// 
    /// # Returns
    /// A configured DialogClient ready to start processing
    ///
    /// # Errors
    /// - `ApiError::Configuration` - Invalid configuration parameters
    /// - `ApiError::Internal` - Failed to create dialog manager with global events
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
    /// This constructor provides direct dependency injection for scenarios where you need
    /// full control over the dependencies. It's particularly useful for testing, legacy
    /// integration, or when you have existing infrastructure to integrate with.
    ///
    /// ## When to Use This Pattern
    ///
    /// - **Testing**: When you need to inject mock dependencies
    /// - **Legacy Integration**: When working with existing transaction managers
    /// - **Simple Use Cases**: When you don't need complex event processing
    /// - **Custom Event Handling**: When you want to handle transaction events differently
    ///
    /// ## Production Considerations
    ///
    /// **⚠️ Warning**: This method uses individual transaction event subscriptions which
    /// may not be as reliable as global event subscriptions. For production applications,
    /// consider using `with_global_events()` instead.
    ///
    /// ## Usage Examples
    ///
    /// ### Basic Usage
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::{DialogClient, DialogApi, ClientConfig};
    /// use crate::transaction::TransactionManager;
    /// use std::sync::Arc;
    ///
    /// # async fn basic_usage() -> Result<(), Box<dyn std::error::Error>> {
    /// // Set up transport (your responsibility)
    /// # let transport = unimplemented!(); // Your transport implementation
    /// let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
    /// let config = ClientConfig::new("127.0.0.1:0".parse()?);
    ///
    /// // Create client with dependency injection
    /// let client = DialogClient::with_dependencies(tx_mgr, config).await?;
    /// client.start().await?;
    ///
    /// // Client is ready for use
    /// let stats = client.get_stats().await;
    /// println!("Client initialized: {} active dialogs", stats.active_dialogs);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Testing Pattern
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::{DialogClient, ClientConfig};
    /// use crate::transaction::TransactionManager;
    /// use std::sync::Arc;
    ///
    /// # async fn testing_pattern() -> Result<(), Box<dyn std::error::Error>> {
    /// // Create mock transport for testing
    /// # let mock_transport = unimplemented!(); // Your mock transport
    /// let tx_mgr = Arc::new(TransactionManager::new_sync(mock_transport));
    /// let config = ClientConfig::new("127.0.0.1:0".parse()?)
    ///     .with_from_uri("sip:test@localhost");
    ///
    /// let client = DialogClient::with_dependencies(tx_mgr, config).await?;
    ///
    /// // Test client operations
    /// assert_eq!(client.config().from_uri.as_ref().unwrap(), "sip:test@localhost");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Integration with Existing Infrastructure
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::{DialogClient, ClientConfig, DialogApi};
    /// use crate::transaction::TransactionManager;
    /// use std::sync::Arc;
    ///
    /// # async fn infrastructure_integration(existing_tx_mgr: Arc<TransactionManager>) -> Result<(), Box<dyn std::error::Error>> {
    /// // Use existing transaction manager from your infrastructure
    /// let config = ClientConfig::new("10.0.1.50:5060".parse()?)
    ///     .with_from_uri("sip:service@internal.company.com")
    ///     .with_auth("service", "internal_password");
    ///
    /// let client = DialogClient::with_dependencies(existing_tx_mgr, config).await?;
    ///
    /// // Integrate with existing systems
    /// client.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    /// 
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager with transport
    /// * `config` - Client configuration with network and authentication settings
    /// 
    /// # Returns
    /// A configured DialogClient ready to start processing
    ///
    /// # Errors
    /// - `ApiError::Configuration` - Invalid configuration parameters
    /// - `ApiError::Internal` - Failed to create dialog manager
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
    /// Creates a new SIP dialog and initiates an outgoing call by sending an INVITE request.
    /// This is the primary method for establishing voice/video calls and handles all the
    /// complexity of SIP dialog creation, request generation, and initial negotiation.
    ///
    /// ## Call Flow
    ///
    /// 1. **URI Validation**: Validates and parses the provided SIP URIs
    /// 2. **Dialog Creation**: Creates a new outgoing dialog with unique identifiers
    /// 3. **INVITE Generation**: Builds and sends an INVITE request with optional SDP
    /// 4. **Handle Creation**: Returns a CallHandle for subsequent call operations
    /// 5. **Statistics Update**: Updates internal call tracking metrics
    ///
    /// ## Usage Examples
    ///
    /// ### Basic Call
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    ///
    /// # async fn basic_call(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
    /// let call = client.make_call(
    ///     "sip:alice@company.com",      // Who the call is from
    ///     "sip:bob@partner.com",        // Who to call
    ///     None                          // No SDP offer (late negotiation)
    /// ).await?;
    ///
    /// println!("Call initiated: {}", call.call_id());
    /// // Call is now ringing, wait for response...
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Call with SDP Offer (Early Media Negotiation)
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    ///
    /// # async fn call_with_sdp(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
    /// let sdp_offer = r#"v=0
    /// o=alice 2890844526 2890844527 IN IP4 192.168.1.100
    /// s=Call
    /// c=IN IP4 192.168.1.100
    /// t=0 0
    /// m=audio 5004 RTP/AVP 0
    /// a=rtpmap:0 PCMU/8000"#;
    ///
    /// let call = client.make_call(
    ///     "sip:alice@company.com",
    ///     "sip:bob@partner.com",
    ///     Some(sdp_offer.to_string())
    /// ).await?;
    ///
    /// println!("Call with media offer sent: {}", call.call_id());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Production Call with Error Handling
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::{DialogClient, ApiError};
    /// use tokio::time::{timeout, Duration};
    ///
    /// # async fn production_call(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
    /// // Set a timeout for the call setup
    /// let call_result = timeout(Duration::from_secs(30), async {
    ///     client.make_call(
    ///         "sip:service@company.com",
    ///         "sip:customer@external.com",
    ///         None
    ///     ).await
    /// }).await;
    ///
    /// match call_result {
    ///     Ok(Ok(call)) => {
    ///         println!("Call successful: {}", call.call_id());
    ///         
    ///         // Set up call monitoring
    ///         tokio::spawn(async move {
    ///             if call.is_active().await {
    ///                 println!("Call is active and can be managed");
    ///             }
    ///         });
    ///     },
    ///     Ok(Err(ApiError::Configuration { message })) => {
    ///         eprintln!("Configuration error: {}", message);
    ///         // Fix URIs and retry
    ///     },
    ///     Ok(Err(ApiError::Network { message })) => {
    ///         eprintln!("Network error: {}", message);
    ///         // Check connectivity and retry
    ///     },
    ///     Err(_) => {
    ///         eprintln!("Call setup timed out");
    ///         // Handle timeout scenario
    ///     },
    ///     Ok(Err(e)) => {
    ///         eprintln!("Call failed: {}", e);
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Batch Call Operations
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    ///
    /// # async fn batch_calls(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
    /// let call_targets = vec![
    ///     ("sip:alice@company.com", "sip:customer1@external.com"),
    ///     ("sip:alice@company.com", "sip:customer2@external.com"),
    ///     ("sip:alice@company.com", "sip:customer3@external.com"),
    /// ];
    ///
    /// // Make multiple calls sequentially for simplicity
    /// let mut calls = Vec::new();
    /// for (from, to) in call_targets {
    ///     let call = client.make_call(from, to, None).await?;
    ///     calls.push(call);
    /// }
    /// println!("Successfully initiated {} calls", calls.len());
    ///
    /// // Monitor all calls
    /// for call in calls {
    ///     let info = call.info().await?;
    ///     println!("Call {}: {} -> {} ({})", 
    ///         call.call_id(), info.local_uri, info.remote_uri, info.state);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    /// 
    /// # Arguments
    /// * `from_uri` - Local SIP URI (From header) - who the call is from
    /// * `to_uri` - Remote SIP URI (To header) - who to call  
    /// * `sdp_offer` - Optional SDP offer for media negotiation
    /// 
    /// # Returns
    /// A CallHandle for managing the call lifecycle and operations
    ///
    /// # Errors
    /// - `ApiError::Configuration` - Invalid SIP URIs provided
    /// - `ApiError::Dialog` - Failed to create dialog or send INVITE
    /// - `ApiError::Internal` - Internal dialog manager error
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
    /// Creates a SIP dialog without immediately sending an INVITE or other initial request.
    /// This is useful for advanced scenarios where you need fine-grained control over the
    /// dialog lifecycle, want to send custom requests, or need to prepare dialogs for
    /// specific protocol sequences.
    ///
    /// ## When to Use This Method
    ///
    /// - **Custom Protocol Sequences**: When you need to send non-INVITE initial requests
    /// - **Conditional Call Setup**: When call initiation depends on external factors
    /// - **Advanced Dialog Management**: When you need dialog state before sending requests
    /// - **Testing Scenarios**: When you want to test dialog creation independently
    /// - **Batch Operations**: When preparing multiple dialogs for coordinated operations
    ///
    /// ## Usage Examples
    ///
    /// ### Basic Dialog Creation
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    /// use rvoip_sip_core::Method;
    ///
    /// # async fn basic_dialog(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create dialog without initial request
    /// let dialog = client.create_dialog(
    ///     "sip:alice@company.com",
    ///     "sip:bob@partner.com"
    /// ).await?;
    ///
    /// println!("Dialog created: {}", dialog.id());
    ///
    /// // Now you can send custom requests
    /// dialog.send_request(Method::Options, None).await?;
    /// dialog.send_request(Method::Info, Some("Custom info".to_string())).await?;
    ///
    /// // Check dialog state
    /// let state = dialog.state().await?;
    /// println!("Dialog state: {:?}", state);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Conditional Call Setup
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    /// use rvoip_sip_core::Method;
    ///
    /// # async fn conditional_setup(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create dialog first
    /// let dialog = client.create_dialog(
    ///     "sip:service@company.com",
    ///     "sip:customer@external.com"
    /// ).await?;
    ///
    /// // Check external conditions before proceeding
    /// if check_customer_availability().await? {
    ///     // Send INVITE if customer is available
    ///     dialog.send_request(Method::Invite, Some("SDP offer".to_string())).await?;
    ///     println!("Call initiated for available customer");
    /// } else {
    ///     // Send MESSAGE instead
    ///     dialog.send_request(Method::Message, Some("Customer callback requested".to_string())).await?;
    ///     println!("Message sent to unavailable customer");
    /// }
    /// # Ok(())
    /// # }
    /// # async fn check_customer_availability() -> Result<bool, Box<dyn std::error::Error>> { Ok(true) }
    /// ```
    ///
    /// ### Advanced Protocol Sequences
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    /// use rvoip_sip_core::Method;
    ///
    /// # async fn advanced_protocol(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
    /// let dialog = client.create_dialog(
    ///     "sip:alice@company.com",
    ///     "sip:conference@partner.com"
    /// ).await?;
    ///
    /// // Custom protocol: Send OPTIONS first to check capabilities
    /// let options_tx = dialog.send_request(Method::Options, None).await?;
    /// println!("Sent OPTIONS: {}", options_tx);
    ///
    /// // Wait for response processing (in real code, you'd handle this via events)
    /// tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    ///
    /// // Send SUBSCRIBE for presence information
    /// let subscribe_tx = dialog.send_request(
    ///     Method::Subscribe, 
    ///     Some("Event: presence\r\nExpires: 3600".to_string())
    /// ).await?;
    /// println!("Sent SUBSCRIBE: {}", subscribe_tx);
    ///
    /// // Finally, send INVITE for the actual call
    /// let invite_tx = dialog.send_request(Method::Invite, Some("SDP offer".to_string())).await?;
    /// println!("Sent INVITE: {}", invite_tx);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Batch Dialog Preparation
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    /// use std::future::Future;
    ///
    /// # async fn batch_dialogs(client: DialogClient) -> Result<(), Box<dyn std::error::Error>> {
    /// let targets = vec![
    ///     "sip:customer1@external.com",
    ///     "sip:customer2@external.com", 
    ///     "sip:customer3@external.com",
    /// ];
    ///
    /// // Create multiple dialogs concurrently using join_all from std
    /// let mut dialogs = Vec::new();
    /// for target in targets {
    ///     let dialog = client.create_dialog("sip:service@company.com", target).await?;
    ///     dialogs.push(dialog);
    /// }
    /// println!("Created {} dialogs", dialogs.len());
    ///
    /// // Now you can coordinate operations across all dialogs
    /// for (i, dialog) in dialogs.iter().enumerate() {
    ///     // Stagger call initiation
    ///     tokio::time::sleep(tokio::time::Duration::from_millis(i as u64 * 100)).await;
    ///     dialog.send_request(rvoip_sip_core::Method::Invite, None).await?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    /// 
    /// # Arguments
    /// * `from_uri` - Local SIP URI (From header)
    /// * `to_uri` - Remote SIP URI (To header)
    /// 
    /// # Returns
    /// A DialogHandle for the new dialog
    ///
    /// # Errors
    /// - `ApiError::Configuration` - Invalid SIP URIs provided
    /// - `ApiError::Dialog` - Failed to create dialog
    /// - `ApiError::Internal` - Internal dialog manager error
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
    /// Sends arbitrary SIP methods within an established dialog. This method provides
    /// direct access to SIP protocol operations and is essential for session-core
    /// coordination, custom protocol implementations, and advanced call control.
    ///
    /// ## Supported SIP Methods
    ///
    /// This method can send any SIP method within a dialog context:
    ///
    /// - **INVITE**: Re-INVITE for media changes, call transfers
    /// - **BYE**: Call termination (prefer using `send_bye()` convenience method)
    /// - **UPDATE**: Media parameter updates (RFC 3311)
    /// - **INFO**: Application-specific information (RFC 6086)
    /// - **REFER**: Call transfers and redirections (RFC 3515)
    /// - **NOTIFY**: Event notifications (RFC 3265)
    /// - **MESSAGE**: Instant messaging within dialogs
    /// - **OPTIONS**: Capability queries
    /// - **SUBSCRIBE**: Event subscriptions
    ///
    /// ## Usage Examples
    ///
    /// ### Media Management with UPDATE
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    /// use rvoip_sip_core::Method;
    /// use rvoip_dialog_core::dialog::DialogId;
    ///
    /// # async fn media_update(client: DialogClient, dialog_id: DialogId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Send UPDATE to modify media parameters
    /// let updated_sdp = r#"v=0
    /// o=alice 2890844526 2890844528 IN IP4 192.168.1.100
    /// s=Updated Call
    /// c=IN IP4 192.168.1.100
    /// t=0 0
    /// m=audio 5004 RTP/AVP 0 8
    /// a=rtpmap:0 PCMU/8000
    /// a=rtpmap:8 PCMA/8000"#;
    ///
    /// let tx_key = client.send_request_in_dialog(
    ///     &dialog_id,
    ///     Method::Update,
    ///     Some(bytes::Bytes::from(updated_sdp))
    /// ).await?;
    ///
    /// println!("Sent UPDATE request: {}", tx_key);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Application Information Exchange
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    /// use rvoip_sip_core::Method;
    /// use rvoip_dialog_core::dialog::DialogId;
    ///
    /// # async fn info_exchange(client: DialogClient, dialog_id: DialogId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Send application-specific information
    /// let app_data = r#"Content-Type: application/json
    ///
    /// {
    ///   "action": "screen_share_request",
    ///   "timestamp": "2024-01-15T10:30:00Z",
    ///   "session_id": "abc123"
    /// }"#;
    ///
    /// let tx_key = client.send_request_in_dialog(
    ///     &dialog_id,
    ///     Method::Info,
    ///     Some(bytes::Bytes::from(app_data))
    /// ).await?;
    ///
    /// println!("Sent INFO with application data: {}", tx_key);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Event Notifications
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    /// use rvoip_sip_core::Method;
    /// use rvoip_dialog_core::dialog::DialogId;
    ///
    /// # async fn event_notification(client: DialogClient, dialog_id: DialogId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Send NOTIFY for presence update
    /// let notify_body = r#"Event: presence
    /// Subscription-State: active;expires=3600
    /// Content-Type: application/pidf+xml
    ///
    /// <?xml version="1.0" encoding="UTF-8"?>
    /// <presence xmlns="urn:ietf:params:xml:ns:pidf" entity="sip:alice@company.com">
    ///   <tuple id="tuple1">
    ///     <status><basic>open</basic></status>
    ///     <contact>sip:alice@company.com</contact>
    ///   </tuple>
    /// </presence>"#;
    ///
    /// let tx_key = client.send_request_in_dialog(
    ///     &dialog_id,
    ///     Method::Notify,
    ///     Some(bytes::Bytes::from(notify_body))
    /// ).await?;
    ///
    /// println!("Sent NOTIFY for presence: {}", tx_key);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Call Transfer with REFER
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    /// use rvoip_sip_core::Method;
    /// use rvoip_dialog_core::dialog::DialogId;
    ///
    /// # async fn call_transfer(client: DialogClient, dialog_id: DialogId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Send REFER for call transfer
    /// let refer_body = r#"Refer-To: sip:bob@partner.com
    /// Referred-By: sip:alice@company.com
    /// Contact: sip:alice@company.com
    /// Content-Length: 0"#;
    ///
    /// let tx_key = client.send_request_in_dialog(
    ///     &dialog_id,
    ///     Method::Refer,
    ///     Some(bytes::Bytes::from(refer_body))
    /// ).await?;
    ///
    /// println!("Sent REFER for transfer: {}", tx_key);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// ### Session Coordination Pattern
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::DialogClient;
    /// use rvoip_sip_core::Method;
    /// use rvoip_dialog_core::dialog::DialogId;
    ///
    /// # async fn session_coordination(client: DialogClient, dialog_id: DialogId) -> Result<(), Box<dyn std::error::Error>> {
    /// // Coordinate with session layer for media changes
    /// let media_session = setup_media_session().await?;
    /// let new_sdp = media_session.generate_offer().await?;
    ///
    /// // Send re-INVITE with new media parameters
    /// let tx_key = client.send_request_in_dialog(
    ///     &dialog_id,
    ///     Method::Invite,
    ///     Some(bytes::Bytes::from(new_sdp))
    /// ).await?;
    ///
    /// println!("Sent re-INVITE for media change: {}", tx_key);
    ///
    /// // The response will be handled by session coordination events
    /// # Ok(())
    /// # }
    /// # async fn setup_media_session() -> Result<MediaSession, Box<dyn std::error::Error>> { Ok(MediaSession) }
    /// # struct MediaSession;
    /// # impl MediaSession {
    /// #     async fn generate_offer(&self) -> Result<String, Box<dyn std::error::Error>> { Ok("SDP".to_string()) }
    /// # }
    /// ```
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to send the request within
    /// * `method` - SIP method to send (INVITE, BYE, UPDATE, INFO, etc.)
    /// * `body` - Optional message body (SDP, application data, etc.)
    /// 
    /// # Returns
    /// Transaction key for tracking the request and its response
    ///
    /// # Errors
    /// - `ApiError::Dialog` - Dialog not found or invalid state
    /// - `ApiError::Protocol` - Invalid method for current dialog state
    /// - `ApiError::Internal` - Failed to send request
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