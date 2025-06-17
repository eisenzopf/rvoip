//! Public API for session-core
//!
//! This module provides the main public interface for the session-core crate.
//! Session-core is the central coordination layer for SIP sessions in the rvoip stack.
//! 
//! # Quick Start
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::sync::Arc;
//! 
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // 1. Create a session coordinator with builder pattern
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_local_address("sip:user@192.168.1.100:5060")
//!         .build()
//!         .await?;
//!     
//!     // 2. Start the coordinator to begin accepting calls
//!     SessionControl::start(&coordinator).await?;
//!     
//!     // 3. Make an outgoing call
//!     let session = SessionControl::create_outgoing_call(
//!         &coordinator,
//!         "sip:alice@example.com",
//!         "sip:bob@192.168.1.100",
//!         None  // SDP will be generated automatically
//!     ).await?;
//!     
//!     // 4. Clean shutdown when done
//!     SessionControl::stop(&coordinator).await?;
//!     Ok(())
//! }
//! ```
//! 
//! # Architecture Overview
//! 
//! Session-core provides a unified API for managing SIP sessions. It coordinates between:
//! - **SIP Signaling**: Dialog management, transactions, and protocol handling
//! - **Media Streams**: RTP/RTCP, codecs, and quality monitoring
//! - **Call Control**: High-level operations like hold, transfer, and conference
//! 
//! The API is organized into several key modules:
//! 
//! | Module | Purpose | Key Types |
//! |--------|---------|-----------|
//! | **`types`** | Core data types | `SessionId`, `CallSession`, `CallState`, `IncomingCall` |
//! | **`control`** | Session control operations | `SessionControl` trait |
//! | **`media`** | Media stream management | `MediaControl` trait |
//! | **`handlers`** | Event handling callbacks | `CallHandler` trait |
//! | **`builder`** | Configuration and setup | `SessionManagerBuilder` |
//! | **`bridge`** | 2-party call bridging | `BridgeId`, `BridgeInfo`, `BridgeEvent` |
//! 
//! # Core Concepts
//! 
//! ## SessionCoordinator
//! 
//! The `SessionCoordinator` is the central hub that manages all sessions. It's created
//! via the builder pattern and provides the implementation for all trait methods.
//! 
//! ## Session Lifecycle
//! 
//! ```text
//! Incoming Call:
//! INVITE received → CallHandler.on_incoming_call() → Decision → Active/Rejected
//! 
//! Outgoing Call:
//! create_outgoing_call() → Initiating → Ringing → Active → Terminated
//! ```
//! 
//! # Two Ways to Handle Incoming Calls
//! 
//! Session-core provides flexibility in how you handle incoming calls:
//! 
//! ## 1. Immediate Decision Pattern (Simple Cases)
//! 
//! Make decisions synchronously in the CallHandler callback:
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! 
//! #[derive(Debug)]
//! struct MyHandler;
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for MyHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // Decision made immediately based on simple rules
//!         if is_business_hours() && is_allowed_number(&call.from) {
//!             // Accept and generate SDP answer inline
//!             let sdp = generate_sdp_answer(&call.sdp.unwrap());
//!             CallDecision::Accept(Some(sdp))
//!         } else if is_blacklisted(&call.from) {
//!             CallDecision::Reject("Blocked number".to_string())
//!         } else {
//!             CallDecision::Forward("sip:voicemail@example.com".to_string())
//!         }
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         log::info!("Call {} ended: {}", call.id(), reason);
//!     }
//! }
//! ```
//! 
//! ## 2. Deferred Decision Pattern (Complex Cases)
//! 
//! Defer the decision for asynchronous processing:
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::sync::{Arc, Mutex};
//! 
//! #[derive(Debug)]
//! struct AsyncHandler {
//!     pending_calls: Arc<Mutex<Vec<IncomingCall>>>,
//! }
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for AsyncHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // Store for later processing
//!         self.pending_calls.lock().unwrap().push(call);
//!         CallDecision::Defer
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         // Update call records, statistics, etc.
//!     }
//! }
//! 
//! // Process deferred calls asynchronously
//! async fn process_pending_calls(
//!     coordinator: &Arc<SessionCoordinator>,
//!     pending: &Arc<Mutex<Vec<IncomingCall>>>
//! ) -> Result<()> {
//!     let calls = pending.lock().unwrap().drain(..).collect::<Vec<_>>();
//!     
//!     for call in calls {
//!         // Perform async operations
//!         let user_info = lookup_user_in_database(&call.from).await?;
//!         let routing_rules = get_routing_rules(&call.to).await?;
//!         
//!         if should_accept(&user_info, &routing_rules) {
//!             // Generate SDP answer using MediaControl
//!             let answer = MediaControl::generate_sdp_answer(
//!                 coordinator,
//!                 &call.id,
//!                 &call.sdp.unwrap()
//!             ).await?;
//!             
//!             // Accept the call
//!             SessionControl::accept_incoming_call(
//!                 coordinator,
//!                 &call,
//!                 Some(answer)
//!             ).await?;
//!         } else {
//!             // Reject with appropriate reason
//!             SessionControl::reject_incoming_call(
//!                 coordinator,
//!                 &call,
//!                 "Service unavailable"
//!             ).await?;
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Common Use Cases
//! 
//! ## Basic SIP Server/Softphone
//! ```rust
//! let coordinator = SessionManagerBuilder::new()
//!     .with_sip_port(5060)
//!     .with_handler(Arc::new(AutoAnswerHandler))
//!     .build()
//!     .await?;
//! 
//! SessionControl::start(&coordinator).await?;
//! ```
//! 
//! ## Call Center with Queue
//! ```rust
//! let queue = Arc::new(QueueHandler::new(100));
//! let coordinator = SessionManagerBuilder::new()
//!     .with_sip_port(5060)
//!     .with_handler(queue.clone())
//!     .build()
//!     .await?;
//! 
//! // Process queued calls in background
//! tokio::spawn(async move {
//!     loop {
//!         process_queue_batch(&coordinator, &queue).await;
//!         tokio::time::sleep(Duration::from_millis(100)).await;
//!     }
//! });
//! ```
//! 
//! ## PBX with Routing Rules
//! ```rust
//! let mut router = RoutingHandler::new();
//! router.add_route("sip:support@*", "sip:queue@support.local");
//! router.add_route("sip:sales@*", "sip:queue@sales.local");
//! router.add_route("sip:*@vip.example.com", "sip:priority@queue.local");
//! 
//! let coordinator = SessionManagerBuilder::new()
//!     .with_sip_port(5060)
//!     .with_handler(Arc::new(router))
//!     .build()
//!     .await?;
//! ```
//! 
//! # Bridge Management (2-Party Conferences)
//! 
//! Session-core provides bridge management for connecting two calls:
//! 
//! ```rust
//! // Bridge two active sessions (e.g., customer and agent)
//! let bridge_id = coordinator.bridge_sessions(&session1_id, &session2_id).await?;
//! 
//! // Monitor bridge events
//! let mut events = coordinator.subscribe_to_bridge_events().await;
//! while let Some(event) = events.recv().await {
//!     match event.event_type {
//!         BridgeEventType::ParticipantAdded { .. } => {
//!             log::info!("Participant joined bridge");
//!         }
//!         BridgeEventType::ParticipantRemoved { .. } => {
//!             log::info!("Participant left bridge");
//!         }
//!         BridgeEventType::BridgeDestroyed { .. } => {
//!             log::info!("Bridge ended");
//!             break;
//!         }
//!     }
//! }
//! ```
//! 
//! # Best Practices
//! 
//! 1. **Use the Builder Pattern** - Configure all settings before building
//! 2. **Handle Errors Properly** - All network operations can fail
//! 3. **Monitor Call Quality** - Use MediaControl::get_media_statistics()
//! 4. **Clean Up Resources** - Always call terminate_session() when done
//! 5. **Choose the Right Pattern** - Immediate for simple cases, deferred for complex
//! 6. **Use Type Safety** - Leverage Rust's type system for compile-time checks
//! 
//! # Error Handling
//! 
//! All API methods return `Result<T, SessionError>` for consistent error handling:
//! 
//! ```rust
//! match SessionControl::create_outgoing_call(&coordinator, from, to, None).await {
//!     Ok(session) => {
//!         log::info!("Call created: {}", session.id());
//!     }
//!     Err(SessionError::InvalidUri(uri)) => {
//!         log::error!("Invalid SIP URI: {}", uri);
//!     }
//!     Err(SessionError::TransportError(e)) => {
//!         log::error!("Network error: {}", e);
//!     }
//!     Err(e) => {
//!         log::error!("Call failed: {}", e);
//!     }
//! }
//! ```
//! 
//! # See Also
//! 
//! - [`SessionControl`] - Main control interface
//! - [`MediaControl`] - Media operations
//! - [`CallHandler`] - Incoming call handling
//! - [`SessionManagerBuilder`] - Configuration
//! - [examples/](https://github.com/yourusername/rvoip/tree/main/crates/session-core/examples) - Full examples
//! - [COOKBOOK.md](https://github.com/yourusername/rvoip/blob/main/crates/session-core/COOKBOOK.md) - Recipes

pub mod types;
pub mod handlers;
pub mod builder;
pub mod control;
pub mod media;
pub mod create;
pub mod examples;

// New API modules
pub mod bridge;
pub mod server_types;

// Re-export main types
pub use types::{
    SessionId, CallSession, CallState, IncomingCall, CallDecision, 
    SessionStats, MediaInfo, PreparedCall, CallDirection, TerminationReason,
    SdpInfo, parse_sdp_connection,
};
pub use handlers::CallHandler;
pub use builder::{SessionManagerBuilder, SessionManagerConfig};
pub use control::SessionControl;
pub use media::MediaControl;

// Re-export bridge functionality
pub use bridge::{
    BridgeId, BridgeInfo, BridgeEvent, BridgeEventType,
};

// Re-export server types
pub use server_types::{
    IncomingCallEvent, CallerInfo,
};

// Re-export conference functionality (make it public)
pub use crate::conference::{
    ConferenceManager, ConferenceApi, ConferenceCoordinator,
    ConferenceId, ConferenceConfig, ConferenceEvent,
    ConferenceRoom, ConferenceParticipant,
};

// Re-export error types
pub use crate::errors::{Result, SessionError};

// Type aliases for compatibility with call-engine
pub type Session = CallSession;
pub type ServerSessionManager = SessionCoordinator;
pub type ServerConfig = SessionManagerConfig;
pub type IncomingCallNotification = IncomingCallEvent;

// Re-export create helper function
pub async fn create_full_server_manager(
    transaction_manager: std::sync::Arc<rvoip_transaction_core::TransactionManager>,
    _config: ServerConfig,
) -> Result<std::sync::Arc<ServerSessionManager>> {
    // Use builder to create coordinator with transaction manager
    SessionManagerBuilder::new()
        .build_with_transaction_manager(transaction_manager)
        .await
}

// Re-export the SessionCoordinator as the main entry point
pub use crate::coordinator::SessionCoordinator; 