//! Dialog Server API
//!
//! This module provides a comprehensive server interface for SIP dialog management,
//! designed for building robust SIP servers, proxies, and application servers that
//! handle incoming calls, dialog state management, and advanced SIP operations.
//!
//! ## Overview
//!
//! The DialogServer is the primary interface for server-side SIP operations, providing
//! a modular, scalable architecture that handles the complexities of SIP dialog management
//! while offering powerful features for call handling, media coordination, and protocol
//! extensions.
//!
//! ### Key Features
//!
//! - **Call Handling**: Accept, reject, and manage incoming calls with full control
//! - **Dialog Management**: Complete SIP dialog lifecycle for server scenarios
//! - **Session Integration**: Built-in coordination with session-core for media management
//! - **Request/Response Handling**: Process incoming requests and generate compliant responses
//! - **Auto-Response Modes**: Automatic handling of OPTIONS, REGISTER, and other methods
//! - **Statistics & Monitoring**: Real-time metrics and performance tracking
//! - **Modular Architecture**: Clean separation of concerns across focused submodules
//!
//! ## Architecture Overview
//!
//! The DialogServer is organized into focused submodules for maintainability:
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │             DialogServer                │
//! ├─────────────────────────────────────────┤
//! │ Core           │ Server struct & config │  ← [`core`]
//! │ Call Ops       │ Call lifecycle mgmt    │  ← [`call_operations`]
//! │ Dialog Ops     │ Dialog management      │  ← [`dialog_operations`]
//! │ Response       │ Response building      │  ← [`response_builder`]
//! │ SIP Methods    │ Specialized handlers   │  ← [`sip_methods`]
//! ├─────────────────────────────────────────┤
//! │         DialogManager (shared)          │
//! ├─────────────────────────────────────────┤
//! │       TransactionManager                │
//! ├─────────────────────────────────────────┤
//! │        TransportManager                 │
//! └─────────────────────────────────────────┘
//! ```
//!
//! ## Quick Start
//!
//! ### Basic SIP Server
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi, ServerConfig};
//! use rvoip_dialog_core::events::SessionCoordinationEvent;
//! use rvoip_dialog_core::transaction::TransactionManager;
//! use tokio::sync::mpsc;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Set up dependencies (transport setup omitted for brevity)
//!     # let transport = unimplemented!(); // Mock transport
//!     let tx_mgr = Arc::new(TransactionManager::new_sync(transport));
//!     let config = ServerConfig::new("0.0.0.0:5060".parse()?)
//!         .with_domain("sip.company.com")
//!         .with_auto_options()
//!         .with_auto_register();
//!     
//!     // Create and configure server
//!     let server = DialogServer::with_dependencies(tx_mgr, config).await?;
//!     
//!     // Set up session coordination for call handling
//!     let (session_tx, mut session_rx) = mpsc::channel(100);
//!     server.set_session_coordinator(session_tx).await?;
//!     
//!     // Start the server
//!     server.start().await?;
//!     println!("✅ SIP server listening on 0.0.0.0:5060");
//!     
//!     // Handle incoming calls
//!     tokio::spawn(async move {
//!         while let Some(event) = session_rx.recv().await {
//!             match event {
//!                 SessionCoordinationEvent::IncomingCall { dialog_id, request, .. } => {
//!                     println!("📞 Incoming call: {} from {}", 
//!                         dialog_id, request.from().unwrap().uri());
//!                     // Handle call - see examples below
//!                 },
//!                 _ => {}
//!             }
//!         }
//!     });
//!     
//!     // Keep server running
//!     tokio::signal::ctrl_c().await?;
//!     server.stop().await?;
//!     println!("✅ Server stopped gracefully");
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Usage Patterns
//!
//! ### Pattern 1: Simple Call Server
//!
//! For basic call handling with automatic responses:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi, ServerConfig};
//! use rvoip_dialog_core::events::SessionCoordinationEvent;
//! use rvoip_sip_core::StatusCode;
//! use tokio::sync::mpsc;
//!
//! # async fn simple_server() -> Result<(), Box<dyn std::error::Error>> {
//! # let (tx_mgr, config) = setup_dependencies().await?;
//! let server = DialogServer::with_dependencies(tx_mgr, config).await?;
//! 
//! // Set up call handling
//! let (session_tx, mut session_rx) = mpsc::channel(100);
//! server.set_session_coordinator(session_tx).await?;
//! server.start().await?;
//! 
//! // Simple call handler
//! tokio::spawn(async move {
//!     while let Some(event) = session_rx.recv().await {
//!         match event {
//!             SessionCoordinationEvent::IncomingCall { dialog_id, request, .. } => {
//!                 // Accept calls using handle_invite with the actual request
//!                 if let Ok(call) = server.handle_invite(request, "127.0.0.1:5060".parse().unwrap()).await {
//!                     call.answer(Some("SDP answer".to_string())).await.ok();
//!                     println!("Call {} auto-accepted", dialog_id);
//!                 }
//!             },
//!             SessionCoordinationEvent::CallTerminated { dialog_id, .. } => {
//!                 println!("Call {} ended", dialog_id);
//!             },
//!             _ => {}
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! # async fn setup_dependencies() -> Result<(std::sync::Arc<rvoip_dialog_core::transaction::TransactionManager>, rvoip_dialog_core::api::ServerConfig), std::io::Error> { unimplemented!() }
//! ```
//!
//! ### Pattern 2: Advanced Call Processing
//!
//! For sophisticated call routing and processing:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi};
//! use rvoip_dialog_core::events::SessionCoordinationEvent;
//! use rvoip_sip_core::{StatusCode, Request};
//! use std::collections::HashMap;
//!
//! # async fn advanced_server(server: DialogServer) -> Result<(), Box<dyn std::error::Error>> {
//! let (session_tx, mut session_rx) = mpsc::channel(100);
//! server.set_session_coordinator(session_tx).await?;
//! server.start().await?;
//!
//! // Advanced call processing with routing logic
//! tokio::spawn(async move {
//!     let mut active_calls = HashMap::new();
//!     
//!     while let Some(event) = session_rx.recv().await {
//!         match event {
//!             SessionCoordinationEvent::IncomingCall { dialog_id, request, .. } => {
//!                 let to_uri = request.to().unwrap().uri().to_string();
//!                 
//!                 // Route based on destination
//!                 match route_call(&to_uri).await {
//!                     CallRoute::Accept(sdp_answer) => {
//!                         if let Ok(call) = server.handle_invite(request, "127.0.0.1:5060".parse().unwrap()).await {
//!                             call.answer(Some(sdp_answer)).await.ok();
//!                             active_calls.insert(dialog_id.clone(), CallInfo::new(to_uri));
//!                             println!("✅ Call {} accepted and routed", dialog_id);
//!                         }
//!                     },
//!                     CallRoute::Redirect(_new_target) => {
//!                         // Handle redirect (simplified for doc test)
//!                         println!("↪️ Call {} would be redirected", dialog_id);
//!                     },
//!                     CallRoute::Reject(_reason) => {
//!                         // Handle rejection (simplified for doc test)
//!                         println!("❌ Call {} would be rejected", dialog_id);
//!                     }
//!                 }
//!             },
//!             SessionCoordinationEvent::CallTerminated { dialog_id, .. } => {
//!                 active_calls.remove(&dialog_id);
//!                 println!("📞 Call {} terminated", dialog_id);
//!             },
//!             _ => {}
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! # enum CallRoute { Accept(String), Redirect(String), Reject(String) }
//! # async fn route_call(to_uri: &str) -> CallRoute { CallRoute::Accept("SDP".to_string()) }
//! # struct CallInfo;
//! # impl CallInfo { fn new(uri: String) -> Self { Self } }
//! # use tokio::sync::mpsc;
//! ```
//!
//! ### Pattern 3: Protocol Extension Server
//!
//! For servers implementing custom SIP extensions:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::DialogServer;
//! use rvoip_sip_core::{Method, StatusCode};
//! use rvoip_dialog_core::dialog::DialogId;
//!
//! # async fn protocol_server(server: DialogServer) -> Result<(), Box<dyn std::error::Error>> {
//! // Handle custom SIP methods and extensions
//! 
//! // Custom NOTIFY handler for presence
//! async fn handle_notify(server: &DialogServer, dialog_id: &DialogId, body: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
//!     if let Some(presence_data) = body {
//!         println!("📍 Presence update: {}", presence_data);
//!         
//!         // Process presence information
//!         let presence_status = parse_presence(&presence_data)?;
//!         update_presence_database(&dialog_id, presence_status).await?;
//!         
//!         // Acknowledgment would be sent via proper transaction handling
//!         println!("✅ Presence processed for dialog {}", dialog_id);
//!     }
//!     Ok(())
//! }
//!
//! // Custom INFO handler for application data
//! async fn handle_info(server: &DialogServer, dialog_id: &DialogId, body: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
//!     if let Some(app_data) = body {
//!         println!("📱 Application data: {}", app_data);
//!         
//!         // Process application-specific information
//!         match parse_app_data(&app_data)? {
//!             AppData::ScreenShare { session_id } => {
//!                 setup_screen_share(dialog_id, &session_id).await?;
//!                 println!("🖥️ Screen share initiated for {}", dialog_id);
//!             },
//!             AppData::FileTransfer { file_id, size } => {
//!                 initiate_file_transfer(dialog_id, &file_id, size).await?;
//!                 println!("📁 File transfer started for {}", dialog_id);
//!             },
//!             _ => {
//!                 println!("❓ Unknown app data type for {}", dialog_id);
//!             }
//!         }
//!     }
//!     Ok(())
//! }
//! # Ok(())
//! # }
//! # fn parse_presence(data: &str) -> Result<String, Box<dyn std::error::Error>> { Ok("online".to_string()) }
//! # async fn update_presence_database(dialog_id: &DialogId, status: String) -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
//! # enum AppData { ScreenShare { session_id: String }, FileTransfer { file_id: String, size: u64 } }
//! # fn parse_app_data(data: &str) -> Result<AppData, Box<dyn std::error::Error>> { Ok(AppData::ScreenShare { session_id: "123".to_string() }) }
//! # async fn setup_screen_share(dialog_id: &DialogId, session_id: &str) -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
//! # async fn initiate_file_transfer(dialog_id: &DialogId, file_id: &str, size: u64) -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
//! ```
//!
//! ## Configuration Patterns
//!
//! ### Production Server Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::ServerConfig;
//! use std::time::Duration;
//!
//! let config = ServerConfig::new("0.0.0.0:5060".parse().unwrap())
//!     .with_domain("sip.production.com")
//!     .with_auto_options()
//!     .with_auto_register();
//!
//! // Customize for high-load production
//! let mut prod_config = config;
//! prod_config.dialog = prod_config.dialog
//!     .with_timeout(Duration::from_secs(300))
//!     .with_max_dialogs(100000)
//!     .with_user_agent("ProductionSIP/1.0")
//!     .without_auto_cleanup(); // Manual control for performance
//! ```
//!
//! ### Development Server Configuration
//!
//! ```rust
//! use rvoip_dialog_core::api::ServerConfig;
//! use std::time::Duration;
//!
//! let dev_config = ServerConfig::new("127.0.0.1:5060".parse().unwrap())
//!     .with_domain("localhost")
//!     .with_auto_options();
//!
//! // Fast timeouts for development
//! let mut test_config = dev_config;
//! test_config.dialog = test_config.dialog
//!     .with_timeout(Duration::from_secs(30))
//!     .with_user_agent("DevSIP/1.0");
//! ```
//!
//! ## Error Handling Strategies
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, ApiError};
//! use rvoip_sip_core::StatusCode;
//! use rvoip_dialog_core::dialog::DialogId;
//!
//! async fn robust_call_handler(server: &DialogServer, dialog_id: &DialogId) -> Result<(), Box<dyn std::error::Error>> {
//!     // This example shows error handling patterns, but actual implementation would need proper request/transaction context
//!     println!("🔄 Processing call for dialog {}", dialog_id);
//!     
//!     // In a real implementation, you would:
//!     // 1. Get the original INVITE request from transaction context
//!     // 2. Use handle_invite() with the actual request
//!     // 3. Use proper transaction keys for responses
//!     
//!     match server.get_dialog_info(dialog_id).await {
//!         Ok(dialog_info) => {
//!             println!("✅ Dialog {} found: {} -> {}", dialog_id, dialog_info.local_uri, dialog_info.remote_uri);
//!             // Handle successful dialog case
//!         },
//!         Err(ApiError::Dialog { message }) => {
//!             eprintln!("Dialog error for {}: {}", dialog_id, message);
//!             // Dialog not found - would send 481 Call/Transaction Does Not Exist in real scenario
//!         },
//!         Err(ApiError::Protocol { message }) => {
//!             eprintln!("Protocol error for {}: {}", dialog_id, message);
//!             // Protocol error - would send 400 Bad Request in real scenario
//!         },
//!         Err(e) => {
//!             eprintln!("Internal error for {}: {}", dialog_id, e);
//!             // Internal error - would send 500 Server Internal Error in real scenario
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! ## Integration Examples
//!
//! ### Media Server Integration
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi};
//! use rvoip_dialog_core::events::SessionCoordinationEvent;
//! use tokio::sync::mpsc;
//!
//! # async fn media_integration() -> Result<(), Box<dyn std::error::Error>> {
//! # let (tx_mgr, config) = setup_dependencies().await?;
//! let server = DialogServer::with_dependencies(tx_mgr, config).await?;
//! let (session_tx, mut session_rx) = mpsc::channel(100);
//! server.set_session_coordinator(session_tx).await?;
//! server.start().await?;
//!
//! tokio::spawn(async move {
//!     while let Some(event) = session_rx.recv().await {
//!         match event {
//!             SessionCoordinationEvent::IncomingCall { dialog_id, request, .. } => {
//!                 // Extract SDP offer from incoming INVITE
//!                 if let Some(offer_sdp) = extract_sdp_from_request(&request) {
//!                     // Set up media session
//!                     match setup_media_session(offer_sdp).await {
//!                         Ok(answer_sdp) => {
//!                             // Accept call with SDP answer
//!                             if let Ok(call) = server.handle_invite(request, "127.0.0.1:5060".parse().unwrap()).await {
//!                                 call.answer(Some(answer_sdp)).await.ok();
//!                                 println!("🎵 Media session established for {}", dialog_id);
//!                             }
//!                         },
//!                         Err(e) => {
//!                             eprintln!("❌ Media setup failed: {}", e);
//!                             // Would reject call in real scenario with proper transaction handling
//!                         }
//!                     }
//!                 }
//!             },
//!             SessionCoordinationEvent::CallTerminated { dialog_id, .. } => {
//!                 // Clean up media resources
//!                 cleanup_media_session(&dialog_id).await.ok();
//!                 println!("🔇 Media session cleaned up for {}", dialog_id);
//!             },
//!             _ => {}
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! # use rvoip_sip_core::Request;
//! # fn extract_sdp_from_request(request: &Request) -> Option<String> { Some("SDP".to_string()) }
//! # async fn setup_media_session(offer: String) -> Result<String, Box<dyn std::error::Error + Send + Sync>> { Ok("answer".to_string()) }
//! # async fn cleanup_media_session(dialog_id: &rvoip_dialog_core::dialog::DialogId) -> Result<(), Box<dyn std::error::Error + Send + Sync>> { Ok(()) }
//! # async fn setup_dependencies() -> Result<(std::sync::Arc<rvoip_dialog_core::transaction::TransactionManager>, rvoip_dialog_core::api::ServerConfig), std::io::Error> { unimplemented!() }
//! ```
//!
//! ### Load Balancer Integration
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi};
//! use rvoip_sip_core::StatusCode;
//! use std::sync::atomic::{AtomicU64, Ordering};
//! use std::sync::Arc;
//!
//! # async fn load_balancer_integration() -> Result<(), Box<dyn std::error::Error>> {
//! # let (tx_mgr, config) = setup_dependencies().await?;
//! let server = DialogServer::with_dependencies(tx_mgr, config).await?;
//! let call_counter = Arc::new(AtomicU64::new(0));
//! let max_calls = 1000; // Server capacity limit
//!
//! let (session_tx, mut session_rx) = tokio::sync::mpsc::channel(100);
//! server.set_session_coordinator(session_tx).await?;
//! server.start().await?;
//!
//! let counter_clone = call_counter.clone();
//! tokio::spawn(async move {
//!     while let Some(event) = session_rx.recv().await {
//!         match event {
//!             rvoip_dialog_core::events::SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
//!                 let current_calls = counter_clone.load(Ordering::Relaxed);
//!                 
//!                 if current_calls >= max_calls {
//!                     // Server at capacity - redirect to another server
//!                     println!("📊 Server at capacity, would redirect call {}", dialog_id);
//!                 } else {
//!                     // Accept the call (simplified for doc test)
//!                     println!("📞 Call {} would be accepted ({}/{})", dialog_id, current_calls + 1, max_calls);
//!                     counter_clone.fetch_add(1, Ordering::Relaxed);
//!                 }
//!             },
//!             rvoip_dialog_core::events::SessionCoordinationEvent::CallTerminated { dialog_id, .. } => {
//!                 counter_clone.fetch_sub(1, Ordering::Relaxed);
//!                 let remaining = counter_clone.load(Ordering::Relaxed);
//!                 println!("📞 Call {} ended ({}/{})", dialog_id, remaining, max_calls);
//!             },
//!             _ => {}
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! # async fn setup_dependencies() -> Result<(std::sync::Arc<rvoip_dialog_core::transaction::TransactionManager>, rvoip_dialog_core::api::ServerConfig), std::io::Error> { unimplemented!() }
//! ```
//!
//! ## Monitoring & Statistics
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::{DialogServer, DialogApi};
//! use tokio::time::{interval, Duration};
//!
//! # async fn monitoring_example(server: DialogServer) -> Result<(), Box<dyn std::error::Error>> {
//! // Set up periodic monitoring
//! let mut monitor_interval = interval(Duration::from_secs(60));
//!
//! tokio::spawn(async move {
//!     loop {
//!         monitor_interval.tick().await;
//!         
//!         let stats = server.get_stats().await;
//!         println!("=== Server Statistics ===");
//!         println!("Active dialogs: {}", stats.active_dialogs);
//!         println!("Total dialogs: {}", stats.total_dialogs);
//!         println!("Success rate: {:.1}%", 
//!             100.0 * stats.successful_calls as f64 / (stats.successful_calls + stats.failed_calls) as f64);
//!         println!("Average call duration: {:.1}s", stats.avg_call_duration);
//!
//!         // Health check
//!         if stats.active_dialogs > 5000 {
//!             println!("⚠️ High dialog count - consider scaling");
//!         }
//!         
//!         // List active dialogs for debugging
//!         let active_dialogs = server.list_active_dialogs().await;
//!         if active_dialogs.len() > 100 {
//!             println!("📋 {} active dialogs (showing first 5):", active_dialogs.len());
//!             for dialog_id in active_dialogs.iter().take(5) {
//!                 if let Ok(info) = server.get_dialog_info(dialog_id).await {
//!                     println!("  Dialog {}: {} -> {} ({})", 
//!                         dialog_id, info.local_uri, info.remote_uri, info.state);
//!                 }
//!             }
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! ```
//!
//! ## Best Practices
//!
//! 1. **Use Session Coordination**: Always set up session event handling for robust call management
//! 2. **Configure Auto-Responses**: Enable auto-OPTIONS and auto-REGISTER for standard compliance
//! 3. **Implement Proper Error Handling**: Use specific error types and appropriate SIP status codes
//! 4. **Monitor Server Health**: Track statistics and implement alerting for high loads
//! 5. **Scale Horizontally**: Use load balancing and capacity limits for production deployments
//! 6. **Validate Configuration**: Always call `validate()` on configurations before use
//! 7. **Clean Shutdown**: Implement graceful shutdown with proper resource cleanup
//! 8. **Security Considerations**: Implement authentication, authorization, and rate limiting
//!
//! ## Thread Safety
//!
//! DialogServer is designed to be thread-safe and can handle concurrent operations:
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::DialogServer;
//! use std::sync::Arc;
//!
//! # async fn thread_safety(server: DialogServer) -> Result<(), Box<dyn std::error::Error>> {
//! let server = Arc::new(server);
//!
//! // Spawn multiple tasks handling different aspects
//! let server1 = server.clone();
//! let call_handler = tokio::spawn(async move {
//!     // Handle incoming calls
//! });
//!
//! let server2 = server.clone();
//! let monitoring_task = tokio::spawn(async move {
//!     // Monitor server statistics
//! });
//!
//! let server3 = server.clone();
//! let protocol_handler = tokio::spawn(async move {
//!     // Handle custom protocol extensions
//! });
//!
//! // All tasks can safely use the server concurrently
//! tokio::try_join!(call_handler, monitoring_task, protocol_handler)?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Submodules
//!
//! - [`core`]: Core server struct, constructors, and configuration management
//! - [`call_operations`]: Call lifecycle management (handle, accept, reject, terminate)
//! - [`dialog_operations`]: Dialog management operations (create, query, list, terminate)  
//! - [`response_builder`]: Response building and sending functionality with SIP compliance
//! - [`sip_methods`]: Specialized SIP method handlers (BYE, REFER, NOTIFY, UPDATE, INFO)

pub mod core;
pub mod call_operations;
pub mod dialog_operations;
pub mod response_builder;
pub mod sip_methods;

// Re-export the main types for external use
pub use core::{DialogServer, ServerStats};