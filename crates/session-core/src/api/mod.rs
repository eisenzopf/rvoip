//! Public API for session-core
//!
//! This module provides the main public interface for the session-core crate.
//! Session-core is the central coordination layer for SIP sessions in the rvoip stack.
//! 
//! # Module Organization
//! 
//! The API is organized into several specialized modules, each serving a specific purpose:
//! 
//! ## Core Types (`types.rs`)
//! 
//! Fundamental data types used throughout the API:
//! - [`SessionId`] - Unique identifier for sessions
//! - [`CallSession`] - Complete session information
//! - [`CallState`] - Session state machine (Initiating, Ringing, Active, etc.)
//! - [`IncomingCall`] - Incoming call representation
//! - [`CallDecision`] - Decision types for incoming calls
//! - [`MediaInfo`] - Media stream information
//! - [`SessionStats`] - Call statistics and quality metrics
//! 
//! ## Session Control (`control.rs`)
//! 
//! The [`SessionControl`] trait provides high-level call control operations:
//! - Creating outgoing calls
//! - Accepting/rejecting incoming calls
//! - Terminating sessions
//! - Hold/resume operations
//! - DTMF sending
//! - Session queries and monitoring
//! 
//! ## Media Management (`media.rs`)
//! 
//! The [`MediaControl`] trait handles all media-related operations:
//! - SDP offer/answer generation
//! - Media session creation and updates
//! - Codec negotiation
//! - RTP/RTCP statistics
//! - Audio quality monitoring
//! - Media hold/resume
//! 
//! ## Event Handlers (`handlers.rs`)
//! 
//! The [`CallHandler`] trait for implementing call event callbacks:
//! - `on_incoming_call()` - Handle incoming calls with immediate or deferred decisions
//! - `on_call_established()` - Called when calls become active
//! - `on_call_ended()` - Cleanup when calls terminate
//! - `on_call_failed()` - Handle call failures
//! 
//! ## Configuration (`builder.rs`)
//! 
//! The [`SessionManagerBuilder`] provides a fluent interface for configuration:
//! - Network settings (SIP port, local address)
//! - Media port ranges
//! - STUN/NAT traversal
//! - Handler registration
//! - SIP client features
//! 
//! ## Bridge Management (`bridge.rs`)
//! 
//! Two-party call bridging functionality:
//! - [`BridgeId`] - Unique bridge identifier
//! - [`BridgeInfo`] - Bridge state and participants
//! - [`BridgeEvent`] - Real-time bridge events
//! - Bridge creation, destruction, and monitoring
//! 
//! ## SIP Client Operations (`client.rs`)
//! 
//! The [`SipClient`] trait for non-session SIP operations:
//! - REGISTER - Endpoint registration
//! - OPTIONS - Capability discovery and keepalive
//! - MESSAGE - Instant messaging
//! - SUBSCRIBE/NOTIFY - Event subscriptions
//! - Raw SIP request sending
//! 
//! ## Call Creation Helpers (`create.rs`)
//! 
//! Convenience functions for different call scenarios:
//! - Simple outgoing calls
//! - Calls with custom SDP
//! - Early media handling
//! - Prepared call patterns
//! 
//! ## Server Integration (`server.rs` & `server_types.rs`)
//! 
//! Types and utilities for server implementations:
//! - [`IncomingCallEvent`] - Enhanced incoming call information
//! - [`CallerInfo`] - Detailed caller information
//! - Server-side session management
//! - Integration with transaction layer
//! 
//! ## Code Examples (`examples.rs`)
//! 
//! Ready-to-use implementations of common patterns:
//! - Auto-answer handler
//! - Call queue handler
//! - Routing handler
//! - Business hours handler
//! - Composite handler patterns
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
//! | **`client`** | Client-related operations | `SipClient`, `RegistrationHandle`, `SipResponse`, `SubscriptionHandle` |
//! | **`create`** | Call creation helpers | Convenience functions for common patterns |
//! | **`server`** | Server integration | Server-specific utilities |
//! | **`server_types`** | Server data types | `IncomingCallEvent`, `CallerInfo` |
//! | **`examples`** | Example handlers | Pre-built handlers for common use cases |
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
//! // Mock helper functions for the example
//! fn is_business_hours() -> bool { true }
//! fn is_allowed_number(from: &str) -> bool { !from.contains("spam") }
//! fn is_blacklisted(from: &str) -> bool { from.contains("blocked") }
//! fn generate_sdp_answer(offer: &str) -> String {
//!     // Simple mock answer
//!     format!("v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n")
//! }
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for MyHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // Decision made immediately based on simple rules
//!         if is_business_hours() && is_allowed_number(&call.from) {
//!             // Accept and generate SDP answer inline
//!             if let Some(ref offer) = call.sdp {
//!                 let sdp = generate_sdp_answer(offer);
//!                 CallDecision::Accept(Some(sdp))
//!             } else {
//!                 // No SDP to generate answer from
//!                 CallDecision::Accept(None)
//!             }
//!         } else if is_blacklisted(&call.from) {
//!             CallDecision::Reject("Blocked number".to_string())
//!         } else {
//!             CallDecision::Forward("sip:voicemail@example.com".to_string())
//!         }
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         println!("Call {} ended: {}", call.id(), reason);
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
//! ) -> std::result::Result<(), Box<dyn std::error::Error>> {
//!     let calls = pending.lock().unwrap().drain(..).collect::<Vec<_>>();
//!     
//!     for call in calls {
//!         // In a real implementation, you would:
//!         // 1. Check user permissions/authentication
//!         // 2. Apply routing rules
//!         // 3. Check business hours or other policies
//!         
//!         // For this example, we'll accept all calls with a simple SDP
//!         if call.sdp.is_some() {
//!             // Generate SDP answer using MediaControl
//!             let answer = MediaControl::generate_sdp_answer(
//!                 coordinator,
//!                 &call.id,
//!                 call.sdp.as_ref().unwrap()
//!             ).await?;
//!             
//!             // Accept the call
//!             SessionControl::accept_incoming_call(
//!                 coordinator,
//!                 &call,
//!                 Some(answer)
//!             ).await?;
//!         } else {
//!             // Reject calls without SDP
//!             SessionControl::reject_incoming_call(
//!                 coordinator,
//!                 &call,
//!                 "No SDP offer"
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
//! use rvoip_session_core::{SessionManagerBuilder, SessionControl};
//! use rvoip_session_core::examples::AutoAnswerHandler;
//! use std::sync::Arc;
//! 
//! async fn setup_basic_server() -> Result<(), Box<dyn std::error::Error>> {
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_handler(Arc::new(AutoAnswerHandler))
//!         .build()
//!         .await?;
//! 
//!     SessionControl::start(&coordinator).await?;
//!     Ok(())
//! }
//! ```
//! 
//! ## Call Center with Queue
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder, SessionControl};
//! use rvoip_session_core::api::handlers::QueueHandler;
//! use std::sync::Arc;
//! use std::time::Duration;
//! 
//! async fn setup_call_center() -> Result<(), Box<dyn std::error::Error>> {
//!     let queue = Arc::new(QueueHandler::new(100));
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_handler(queue.clone())
//!         .build()
//!         .await?;
//! 
//!     // In a real implementation, you would process queued calls in background:
//!     // - Check queue.dequeue() periodically
//!     // - Accept/reject calls based on agent availability
//!     // - Use SessionControl::accept_incoming_call() or reject_incoming_call()
//!     
//!     Ok(())
//! }
//! ```
//! 
//! ## PBX with Routing Rules
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder};
//! use rvoip_session_core::api::handlers::RoutingHandler;
//! use std::sync::Arc;
//! 
//! async fn setup_pbx() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut router = RoutingHandler::new();
//!     router.add_route("sip:support@*", "sip:queue@support.local");
//!     router.add_route("sip:sales@*", "sip:queue@sales.local");
//!     router.add_route("sip:*@vip.example.com", "sip:priority@queue.local");
//!     
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_handler(Arc::new(router))
//!         .build()
//!         .await?;
//!     
//!     Ok(())
//! }
//! ```
//! 
//! ## SIP Client Operations
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder, SipClient};
//! 
//! async fn sip_client_example() -> Result<(), Box<dyn std::error::Error>> {
//!     // Enable SIP client features
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5061)
//!         .enable_sip_client()
//!         .build()
//!         .await?;
//!     
//!     // Register with a SIP server
//!     let registration = coordinator.register(
//!         "sip:registrar.example.com",
//!         "sip:alice@example.com",
//!         "sip:alice@192.168.1.100:5061",
//!         3600  // 1 hour
//!     ).await?;
//!     
//!     // Send an instant message
//!     let response = coordinator.send_message(
//!         "sip:bob@example.com",
//!         "Hello from session-core!",
//!         Some("text/plain")
//!     ).await?;
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Bridge Management (2-Party Conferences)
//! 
//! Session-core provides bridge management for connecting two calls:
//! 
//! ```rust
//! use rvoip_session_core::{SessionCoordinator, SessionId, BridgeEvent};
//! use std::sync::Arc;
//! 
//! async fn bridge_example(
//!     coordinator: Arc<SessionCoordinator>,
//!     session1_id: SessionId,
//!     session2_id: SessionId
//! ) -> Result<(), Box<dyn std::error::Error>> {
//!     // Bridge two active sessions (e.g., customer and agent)
//!     let bridge_id = coordinator.bridge_sessions(&session1_id, &session2_id).await?;
//!     
//!     // Monitor bridge events
//!     let mut events = coordinator.subscribe_to_bridge_events().await;
//!     while let Some(event) = events.recv().await {
//!         match event {
//!             BridgeEvent::ParticipantAdded { bridge_id, session_id } => {
//!                 println!("Session {} joined bridge {}", session_id, bridge_id);
//!             }
//!             BridgeEvent::ParticipantRemoved { bridge_id, session_id, reason } => {
//!                 println!("Session {} left bridge {}: {}", session_id, bridge_id, reason);
//!             }
//!             BridgeEvent::BridgeDestroyed { bridge_id } => {
//!                 println!("Bridge {} ended", bridge_id);
//!                 break;
//!             }
//!         }
//!     }
//!     
//!     Ok(())
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
//! use rvoip_session_core::{SessionCoordinator, SessionControl, SessionError};
//! use std::sync::Arc;
//! 
//! async fn handle_errors(
//!     coordinator: Arc<SessionCoordinator>,
//!     from: &str,
//!     to: &str
//! ) {
//!     match SessionControl::create_outgoing_call(&coordinator, from, to, None).await {
//!         Ok(session) => {
//!             println!("Call created: {}", session.id());
//!         }
//!         Err(SessionError::InvalidUri(uri)) => {
//!             eprintln!("Invalid SIP URI: {}", uri);
//!         }
//!         Err(SessionError::NetworkError(e)) => {
//!             eprintln!("Network error: {}", e);
//!         }
//!         Err(e) => {
//!             eprintln!("Call failed: {}", e);
//!         }
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
//! - [`SipClient`] - Non-session SIP operations
//! - [examples/](https://github.com/yourusername/rvoip/tree/main/crates/session-core/examples) - Full examples
//! - [COOKBOOK.md](https://github.com/yourusername/rvoip/blob/main/crates/session-core/COOKBOOK.md) - Recipes
//! - [SIP_CLIENT_DESIGN.md](https://github.com/yourusername/rvoip/blob/main/crates/session-core/SIP_CLIENT_DESIGN.md) - SipClient design

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
pub mod server;
pub mod client;

// Re-export main types
pub use types::{
    SessionId, CallSession, CallState, IncomingCall, CallDecision, 
    SessionStats, MediaInfo, PreparedCall, CallDirection, TerminationReason,
    SdpInfo, parse_sdp_connection,
};
pub use handlers::CallHandler;
pub use builder::{SessionManagerBuilder, SessionManagerConfig, MediaConfig};
pub use control::SessionControl;
pub use media::MediaControl;
pub use client::{SipClient, RegistrationHandle, SipResponse, SubscriptionHandle};

// Re-export bridge functionality
pub use bridge::{
    BridgeId, BridgeInfo, BridgeEvent, BridgeEventType,
};

// Re-export server types
pub use server_types::{
    IncomingCallEvent, CallerInfo,
};

// Re-export server configuration types from server module
pub use server::{
    ServerConfig, TransportProtocol, ServerSessionManager,
    create_full_server_manager,
};

// Re-export conference functionality (make it public)
pub use crate::conference::{
    ConferenceManager, ConferenceApi, ConferenceCoordinator,
    ConferenceId, ConferenceConfig, ConferenceEvent,
    ConferenceRoom, ConferenceParticipant,
};

// Re-export error types
pub use crate::errors::{Result, SessionError};

// Re-export event types for CallHandler use
pub use crate::manager::events::{
    MediaQualityAlertLevel,
    MediaFlowDirection,
    WarningCategory,
};

// Re-export statistics types from media module
pub use crate::media::stats::{
    CallStatistics,
    MediaStatistics as MediaSessionStats,  // Alias for compatibility
    RtpSessionStats,
    QualityMetrics,
    QualityThresholds,
};

// Type aliases for compatibility with call-engine
pub type Session = CallSession;
pub type IncomingCallNotification = IncomingCallEvent;

// Re-export the SessionCoordinator as the main entry point
pub use crate::coordinator::SessionCoordinator; 